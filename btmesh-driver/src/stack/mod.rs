use crate::stack::unprovisioned::UnprovisionedStack;
use crate::storage::provisioned::ProvisionedConfiguration;
use crate::storage::unprovisioned::UnprovisionedConfiguration;
use crate::storage::Configuration;
use crate::util::deadline::DeadlineFuture;
use crate::{DeviceState, ProvisionedStack, Sequence};
use btmesh_common::Uuid;
use core::future::{pending, Future};

pub mod provisioned;
pub mod unprovisioned;

#[allow(clippy::large_enum_variant)]
pub enum Stack {
    None,
    Unprovisioned {
        stack: UnprovisionedStack,
        uuid: Uuid,
    },
    Provisioned {
        stack: ProvisionedStack,
        sequence: Sequence,
    },
}

impl Stack {
    pub fn device_state(&self) -> Option<DeviceState> {
        match self {
            Stack::None => None,
            Stack::Unprovisioned { stack, uuid, .. } => Some(DeviceState::Unprovisioned {
                uuid: *uuid,
                in_progress: stack.in_progress(),
            }),
            Stack::Provisioned { .. } => Some(DeviceState::Provisioned),
        }
    }

    pub fn next_beacon_deadline(&self) -> Option<DeadlineFuture<'_>> {
        match self {
            Stack::None => None,
            Stack::Unprovisioned { stack, .. } => stack.next_beacon_deadline(),
            Stack::Provisioned { stack, .. } => stack.next_beacon_deadline(),
        }
    }

    pub fn next_retransmit(&self) -> Option<impl Future<Output = ()> + '_> {
        if let Stack::None = self {
            return None;
        }
        Some(async move {
            match self {
                Stack::None => pending().await,
                Stack::Unprovisioned { stack, .. } => {
                    if let Some(fut) = stack.next_retransmit() {
                        fut.await
                    } else {
                        pending().await
                    }
                }
                Stack::Provisioned { stack, .. } => {
                    if let Some(fut) = stack.next_retransmit() {
                        fut.await
                    } else {
                        pending().await
                    }
                }
            }
        })
    }
}

impl TryFrom<&Stack> for Configuration {
    type Error = ();

    fn try_from(stack: &Stack) -> Result<Self, Self::Error> {
        match stack {
            Stack::None => Err(()),
            Stack::Unprovisioned { uuid, .. } => {
                Ok(UnprovisionedConfiguration { uuid: *uuid }.into())
            }
            Stack::Provisioned { stack, sequence } => Ok(ProvisionedConfiguration {
                network_state: stack.network_state(),
                secrets: stack.secrets().clone(),
                device_info: stack.device_info(),
                sequence: sequence.current(),
                foundation: Default::default(),
            }
            .into()),
        }
    }
}
