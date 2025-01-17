#![cfg_attr(not(test), no_std)]
#![feature(type_alias_impl_trait)]
#![feature(generic_associated_types)]
#![feature(associated_type_defaults)]
#![feature(future_join)]
#![feature(future_poll_fn)]
#![allow(dead_code)]

#[cfg(not(any(feature = "nrf52833", feature = "nrf52840",)))]
compile_error!("No chip feature activated. You must activate exactly one of the following features: nrf52833, nrf52840");

mod advertising;
mod gatt;

mod rng;

mod driver;

pub use btmesh_driver::BluetoothMeshDriver;

#[cfg(feature = "gatt")]
pub use driver::NrfSoftdeviceAdvertisingAndGattDriver as Driver;

#[cfg(not(feature = "gatt"))]
pub use driver::NrfSoftdeviceAdvertisingOnlyDriver as Driver;

