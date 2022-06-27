use crate::provisioned::system::{AccessMetadata, ControlMetadata, KeyHandle, UpperMetadata};
use crate::provisioned::{DriverError, ProvisionedDriver};
use btmesh_common::address::{Address, LabelUuid};
use btmesh_common::crypto::nonce::{ApplicationNonce, DeviceNonce};
use btmesh_common::mic::{SzMic, TransMic};
use btmesh_common::{crypto, Seq, SeqRolloverError};
use btmesh_pdu::access::AccessMessage;
use btmesh_pdu::control::ControlMessage;
use btmesh_pdu::upper::access::UpperAccessPDU;
use btmesh_pdu::upper::UpperPDU;
use btmesh_pdu::Message;
use core::ops::ControlFlow;
use heapless::Vec;
use crate::provisioned::sequence::Sequence;

#[derive(Default)]
pub struct UpperDriver<const N: usize = 20> {
    label_uuids: Vec<Option<LabelUuid>, N>,
}

impl UpperDriver {
}

impl ProvisionedDriver {
    fn add_label_uuid(&mut self, label_uuid: LabelUuid) -> Result<(), DriverError> {
        if let Some(empty_slot) = self
            .upper
            .label_uuids
            .iter_mut()
            .find(|e| matches!(e, None))
        {
            empty_slot.replace(label_uuid);
            Ok(())
        } else {
            Err(DriverError::InsufficientSpace)
        }
    }

    fn remove_label_uuid(&mut self, label_uuid: LabelUuid) {
        self.upper
            .label_uuids
            .iter_mut()
            .filter(|e| {
                if let Some(inner) = e {
                    *inner == label_uuid
                } else {
                    false
                }
            })
            .for_each(|slot| {
                slot.take();
            })
    }

    pub fn process_inbound_upper_pdu(
        &mut self,
        mut pdu: UpperPDU<ProvisionedDriver>,
    ) -> Result<Message<ProvisionedDriver>, DriverError> {
        self.apply_label_uuids(&mut pdu)?;
        match pdu {
            UpperPDU::Access(access) => Ok(self.decrypt_access(access)?.into()),
            UpperPDU::Control(control) => Ok(ControlMessage::new(
                control.opcode(),
                control.parameters(),
                ControlMetadata::from_upper_control_pdu(&control),
            )?
            .into()),
        }
    }

    pub fn process_outbound_message(
        &mut self,
        sequence: &Sequence,
        message: &Message<ProvisionedDriver>,
    ) -> Result<UpperPDU<ProvisionedDriver>, DriverError> {
        match message {
            Message::Access(access) => {
                Ok(self.encrypt_access(sequence, access)?.into())
            }
            Message::Control(control) => {
                todo!()
            }
        }
    }

    /// Apply potential candidate label-uuids if the destination of the PDU
    /// is a virtual-address.
    fn apply_label_uuids(&self, pdu: &mut UpperPDU<ProvisionedDriver>) -> Result<(), DriverError> {
        if let Address::Virtual(virtual_address) = pdu.meta().dst() {
            let result = self.upper.label_uuids.iter().try_for_each(|slot| {
                if let Some(label_uuid) = slot {
                    if label_uuid.virtual_address() == virtual_address {
                        if let Err(err) = pdu.meta_mut().add_label_uuid(*label_uuid) {
                            return ControlFlow::Break(err);
                        }
                    }
                }

                ControlFlow::Continue(())
            });

            match result {
                ControlFlow::Break(err) => Err(err),
                _ => Ok(()),
            }
        } else {
            Ok(())
        }
    }

    fn encrypt_access(
        &mut self,
        sequence: &Sequence,
        message: &AccessMessage<ProvisionedDriver>,
    ) -> Result<UpperAccessPDU<ProvisionedDriver>, DriverError> {
        let seq_zero = sequence.next();

        let mut payload = Vec::<u8, 379>::new();
        message.emit(&mut payload)?;

        match message.meta().key_handle() {
            KeyHandle::Device => {
                let nonce = DeviceNonce::new(
                    SzMic::Bit32,
                    seq_zero,
                    message.meta().src(),
                    message.meta().dst(),
                    message.meta().iv_index(),
                );

                let device_key = self.secrets.device_key();

                let mut bytes = [0; 379];
                let mut transmic = [0; 4];

                crypto::device::encrypt_device_key(device_key, nonce, &mut bytes, &mut transmic)
                    .map_err(|_| DriverError::CryptoError)?;

                let transmic = TransMic::parse(&transmic)?;

                Ok(UpperAccessPDU::new(
                    &payload,
                    transmic,
                    UpperMetadata::from_access_message(message, seq_zero),
                )?)
            }
            KeyHandle::Network(_) => {
                todo!()
            }
            KeyHandle::Application(key_handle) => {
                let nonce = ApplicationNonce::new(
                    SzMic::Bit32,
                    seq_zero,
                    message.meta().src(),
                    message.meta().dst(),
                    message.meta().iv_index(),
                );

                let application_key = self.secrets.application_key(key_handle)?;

                let mut bytes = [0; 379];
                let mut transmic = [0; 4];

                crypto::application::encrypt_application_key(
                    application_key,
                    nonce,
                    &mut bytes,
                    &mut transmic,
                    message.meta().label_uuid()
                )
                .map_err(|_| DriverError::CryptoError)?;

                let transmic = TransMic::parse(&transmic)?;

                Ok(UpperAccessPDU::new(
                    &payload,
                    transmic,
                    UpperMetadata::from_access_message(message, seq_zero),
                )?)
            }
        }
    }

    fn decrypt_access(
        &mut self,
        pdu: UpperAccessPDU<ProvisionedDriver>,
    ) -> Result<AccessMessage<ProvisionedDriver>, DriverError> {
        if let Some(aid) = pdu.meta().aid() {
            // akf=true and an AID was provided.
            let nonce = ApplicationNonce::new(
                pdu.transmic().szmic(),
                pdu.meta().seq(),
                pdu.meta().src(),
                pdu.meta().dst(),
                pdu.meta().iv_index(),
            );

            let mut decrypt_result = None;

            'outer: for application_key_handle in self.secrets.application_keys_by_aid(aid) {
                let application_key = self.secrets.application_key(application_key_handle)?;
                if pdu.meta().label_uuids().is_empty() {
                    let mut bytes = Vec::<_, 380>::from_slice(pdu.payload())
                        .map_err(|_| DriverError::InsufficientSpace)?;
                    if crypto::application::try_decrypt_application_key(
                        application_key,
                        nonce,
                        &mut bytes,
                        pdu.transmic().as_slice(),
                        None,
                    )
                    .is_ok()
                    {
                        decrypt_result.replace((application_key_handle, None, bytes));
                        break 'outer;
                    }
                } else {
                    // try each label-uuid until success.
                    // while this is two nested loops, the probability of
                    // more than a single execution is exceedingly low,
                    // but never zero.
                    for label_uuid in pdu.meta().label_uuids() {
                        let mut bytes = Vec::<_, 380>::from_slice(pdu.payload())
                            .map_err(|_| DriverError::InsufficientSpace)?;
                        if crypto::application::try_decrypt_application_key(
                            application_key,
                            nonce,
                            &mut bytes,
                            pdu.transmic().as_slice(),
                            Some(*label_uuid),
                        )
                        .is_ok()
                        {
                            decrypt_result.replace((
                                application_key_handle,
                                Some(*label_uuid),
                                bytes,
                            ));
                            break 'outer;
                        }
                    }
                }
            }

            if let Some((application_key_handle, label_uuid, bytes)) = decrypt_result {
                return Ok(AccessMessage::parse(
                    &*bytes,
                    AccessMetadata::from_upper_access_pdu(
                        KeyHandle::Application(application_key_handle),
                        label_uuid,
                        &pdu,
                    ),
                )?);
            }
        } else {
            let nonce = DeviceNonce::new(
                pdu.transmic().szmic(),
                pdu.meta().seq(),
                pdu.meta().src(),
                pdu.meta().dst(),
                pdu.meta().iv_index(),
            );

            let device_key = self.secrets.device_key();

            let mut bytes = Vec::<_, 380>::from_slice(pdu.payload())
                .map_err(|_| DriverError::InsufficientSpace)?;
            if crypto::device::try_decrypt_device_key(
                device_key,
                nonce,
                &mut bytes,
                pdu.transmic().as_slice(),
            )
            .is_ok()
            {
                return Ok(AccessMessage::parse(
                    &*bytes,
                    AccessMetadata::from_upper_access_pdu(KeyHandle::Device, None, &pdu),
                )?);
            }
        }

        Err(DriverError::InvalidPDU)
    }
}
