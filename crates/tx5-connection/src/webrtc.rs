use crate::{BackendModule, CloseRecv};
use futures::future::BoxFuture;
use std::io::Result;
use std::sync::Arc;

pub enum WebrtcEvt {
    GeneratedOffer(Vec<u8>),
    GeneratedAnswer(Vec<u8>),
    GeneratedIce(Vec<u8>),
    Message(Vec<u8>),
    Ready,
}

pub trait Webrtc: 'static + Send + Sync {
    fn in_offer(&self, offer: Vec<u8>) -> BoxFuture<'_, Result<()>>;
    fn in_answer(&self, answer: Vec<u8>) -> BoxFuture<'_, Result<()>>;
    fn in_ice(&self, ice: Vec<u8>) -> BoxFuture<'_, Result<()>>;
    fn message(&self, message: Vec<u8>) -> BoxFuture<'_, Result<()>>;
}

pub type DynWebrtc = Arc<dyn Webrtc + 'static + Send + Sync>;

#[cfg(feature = "backend-libdatachannel")]
mod libdatachannel;

#[cfg(feature = "backend-go-pion")]
mod go_pion;

pub fn new_backend_module(
    module: BackendModule,
    is_polite: bool,
    config: Vec<u8>,
    send_buffer: usize,
) -> (DynWebrtc, CloseRecv<WebrtcEvt>) {
    match module {
        #[cfg(feature = "backend-libdatachannel")]
        BackendModule::LibDataChannel => {
            libdatachannel::Webrtc::new(is_polite, config, send_buffer)
        }
        #[cfg(feature = "backend-go-pion")]
        BackendModule::GoPion => {
            go_pion::Webrtc::new(is_polite, config, send_buffer)
        }
    }
}
