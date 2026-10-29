//! Reads the compositor's clipboard without an external clipboard command.
use super::Dock;
use smithay_client_toolkit::{
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::DataSourceHandler,
    },
    delegate_data_device,
    reexports::{
        calloop::channel,
        client::{
            Connection, QueueHandle,
            globals::GlobalList,
            protocol::{
                wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
                wl_data_source::WlDataSource, wl_seat::WlSeat, wl_surface::WlSurface,
            },
        },
    },
};
use std::io::Read;

pub struct Clipboard {
    manager: Option<DataDeviceManagerState>,
    device: Option<DataDevice>,
}
impl Clipboard {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<Dock>) -> Self {
        Self {
            manager: DataDeviceManagerState::bind(globals, qh).ok(),
            device: None,
        }
    }
    pub fn seat(&mut self, qh: &QueueHandle<Dock>, seat: &WlSeat) {
        if let Some(manager) = &self.manager {
            self.device = Some(manager.get_data_device(qh, seat));
        }
    }
    pub fn paste(
        &self,
        sender: channel::Sender<(u64, usize, String)>,
        generation: u64,
        focus: usize,
    ) {
        let Some(offer) = self
            .device
            .as_ref()
            .and_then(|device| device.data().selection_offer())
        else {
            return;
        };
        let mime = offer.with_mime_types(|types| {
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
                .iter()
                .find(|mime| types.iter().any(|t| t == **mime))
                .map(|s| s.to_string())
        });
        if let Some(mime) = mime {
            if let Ok(pipe) = offer.receive(mime) {
                std::thread::spawn(move || {
                    let mut bytes = Vec::new();
                    if pipe.take(16_384).read_to_end(&mut bytes).is_ok() {
                        let _ = sender.send((
                            generation,
                            focus,
                            String::from_utf8_lossy(&bytes).into_owned(),
                        ));
                    }
                });
            }
        }
    }
}
// The dock receives selections but never publishes a source or accepts a drag.
impl DataDeviceHandler for Dock {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}
impl DataOfferHandler for Dock {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}
impl DataSourceHandler for Dock {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: String,
        _: WritePipe,
    ) {
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}
delegate_data_device!(Dock);
