use btmesh_common::address::InvalidAddress;
use btmesh_common::mic::InvalidLength;
use btmesh_common::{InsufficientBuffer, ParseError, SeqRolloverError};
use btmesh_pdu::provisioned::lower::InvalidBlock;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DriverError {
    InvalidState,
    InvalidKeyLength,
    CryptoError,
    InvalidAddress,
    InsufficientSpace,
    InvalidKeyHandle,
    InvalidPDU,
    IncompleteTransaction,
    ParseError(ParseError),
    SeqRolloverError,
}

impl From<InvalidLength> for DriverError {
    fn from(_: InvalidLength) -> Self {
        Self::CryptoError
    }
}

impl From<SeqRolloverError> for DriverError {
    fn from(_: SeqRolloverError) -> Self {
        Self::SeqRolloverError
    }
}

impl From<InsufficientBuffer> for DriverError {
    fn from(_: InsufficientBuffer) -> Self {
        Self::InsufficientSpace
    }
}

impl From<ParseError> for DriverError {
    fn from(inner: ParseError) -> Self {
        Self::ParseError(inner)
    }
}

impl From<InvalidAddress> for DriverError {
    fn from(_: InvalidAddress) -> Self {
        Self::InvalidAddress
    }
}

impl From<InvalidBlock> for DriverError {
    fn from(_: InvalidBlock) -> Self {
        Self::InvalidState
    }
}

impl From<cmac::crypto_mac::InvalidKeyLength> for DriverError {
    fn from(e: cmac::crypto_mac::InvalidKeyLength) -> Self {
        e.into()
    }
}
