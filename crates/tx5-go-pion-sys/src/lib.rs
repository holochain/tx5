#![doc = tx5_core::__doc_header!()]
//! # tx5-go-pion-sys
//!
//! Rust bindings to the go pion webrtc library.
//!
//! Access the go-pion-webrtc api interface using the
//! pub once_cell::sync::Lazy static [API] handle.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::type_complexity)]
#![allow(clippy::drop_non_drop)]

// Link to CoreFoundation / Security on any Apple device.
#[cfg_attr(
    any(target_os = "macos", target_os = "ios", target_os = "tvos"),
    link(name = "CoreFoundation", kind = "framework")
)]
#[cfg_attr(
    any(target_os = "macos", target_os = "ios", target_os = "tvos"),
    link(name = "Security", kind = "framework")
)]
extern "C" {}

/// Re-exported dependencies.
pub mod deps {
    pub use libc;
    pub use once_cell;
    pub use tx5_core;
    pub use tx5_core::deps::*;
}

pub use tx5_core::{Error, ErrorExt, Id, Result};

use once_cell::sync::Lazy;
use std::sync::Arc;

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
const LIB_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/go-pion-webrtc.dylib"));

#[cfg(target_os = "windows")]
const LIB_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/go-pion-webrtc.dll"));

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "windows",
)))]
const LIB_BYTES: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/go-pion-webrtc.so"));

include!(concat!(env!("OUT_DIR"), "/lib_hash.rs"));

/// Constants.
pub mod constants {
    include!(concat!(env!("OUT_DIR"), "/constants.rs"));
}
use constants::*;

#[ouroboros::self_referencing]
struct LibInner {
    _file: std::fs::File,
    lib: libloading::Library,
    #[borrows(lib)]
    // not 100% sure about this, but we never unload the lib,
    // so it's effectively 'static
    #[covariant]
    on_event: libloading::Symbol<
        'this,
        unsafe extern "C" fn(
            // event_cb
            Option<
                unsafe extern "C" fn(
                    *mut libc::c_void, // response_usr
                    usize,             // response_type
                    usize,             // slot_a
                    usize,             // slot_b
                    usize,             // slot_c
                    usize,             // slot_d
                ),
            >,
            *mut libc::c_void, // event_usr
        ) -> *mut libc::c_void,
    >,
    #[borrows(lib)]
    // not 100% sure about this, but we never unload the lib,
    // so it's effectively 'static
    #[covariant]
    call: libloading::Symbol<
        'this,
        unsafe extern "C" fn(
            usize, // call_type
            usize, // slot_a
            usize, // slot_b
            usize, // slot_c
            usize, // slot_d
            // response_cb
            Option<
                unsafe extern "C" fn(
                    *mut libc::c_void, // response_usr
                    usize,             // response_type
                    usize,             // slot_a
                    usize,             // slot_b
                    usize,             // slot_c
                    usize,             // slot_d
                ),
            >,
            *mut libc::c_void, // response_usr
        ),
    >,
}

impl LibInner {
    unsafe fn priv_new() -> Self {
        let mut path_1 =
            dirs::data_local_dir().expect("failed to determine data dir");
        let mut path_2 = dunce::canonicalize(".")
            .expect("failed to canonicalize current dir");

        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
        let ext = ".dylib";
        #[cfg(target_os = "windows")]
        let ext = ".dll";
        #[cfg(not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "windows",
        )))]
        let ext = ".so";

        path_1.push(format!("go-pion-webrtc-{LIB_HASH}{ext}"));
        path_2.push(format!("go-pion-webrtc-{LIB_HASH}{ext}"));

        for path in [&path_1, &path_2] {
            tracing::trace!("check lib file: {path:?}");

            let mut opts = std::fs::OpenOptions::new();

            opts.write(true);
            opts.create_new(true);

            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);

            if let Ok(mut file) = opts.open(path) {
                use std::io::Write;

                file.write_all(LIB_BYTES)
                    .expect("failed to write lib bytes");
                file.flush().expect("failed to flush lib bytes");

                let mut perms = file
                    .metadata()
                    .expect("failed to get lib metadata")
                    .permissions();

                perms.set_readonly(true);
                #[cfg(unix)]
                std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o400);

                file.set_permissions(perms)
                    .expect("failed to set lib permissions");

                tracing::trace!("wrote lib file: {path:?}");
            }

            if let Ok(mut file) =
                std::fs::OpenOptions::new().read(true).open(path)
            {
                use std::io::Read;

                let mut data = Vec::new();
                file.read_to_end(&mut data).expect("failed to read lib");

                use sha2::Digest;
                let mut hasher = sha2::Sha256::new();
                hasher.update(data);
                let hash = base64::encode_config(
                    hasher.finalize(),
                    base64::URL_SAFE_NO_PAD,
                );

                assert_eq!(LIB_HASH, hash);

                let perms = file
                    .metadata()
                    .expect("failed to get lib metadata")
                    .permissions();

                assert!(perms.readonly());

                let lib = libloading::Library::new(path)
                    .expect("failed to load shared");

                tracing::trace!("success correct lib file: {path:?}");

                return LibInnerBuilder {
                    _file: file,
                    lib,
                    on_event_builder: |lib: &libloading::Library| {
                        lib.get(b"OnEvent").expect("failed to load symbol")
                    },
                    call_builder: |lib: &libloading::Library| {
                        lib.get(b"Call").expect("failed to load symbol")
                    },
                }
                .build();
            }
        }
        panic!("invalid lib paths: {path_1:?} {path_2:?}");
    }
}

pub type CallType = usize;
pub type ResponseUsr = *mut libc::c_void;
pub type ResponseType = usize;
pub type SlotA = usize;
pub type SlotB = usize;
pub type SlotC = usize;
pub type SlotD = usize;
pub type ErrorCode = usize;
pub type BufferId = usize;
pub type PeerConId = usize;
pub type DataChanId = usize;
pub type PeerConState = usize;

#[derive(Debug)]
pub enum Event {
    Error(std::io::Error),
    PeerConICECandidate {
        peer_con_id: PeerConId,
        candidate: BufferId,
    },
    PeerConStateChange {
        peer_con_id: PeerConId,
        peer_con_state: PeerConState,
    },
    PeerConDataChan {
        peer_con_id: PeerConId,
        data_chan_id: DataChanId,
    },
    DataChanClose(DataChanId),
    DataChanOpen(DataChanId),
    DataChanMessage {
        data_chan_id: DataChanId,
        buffer_id: BufferId,
    },
}

pub struct Api(LibInner);

impl Api {
    fn priv_new() -> Self {
        Self(unsafe { LibInner::priv_new() })
    }

    pub unsafe fn on_event<Cb>(&self, cb: Cb)
    where
        Cb: Fn(Event) + 'static + Send + Sync,
    {
        type DynCb = Box<Arc<dyn Fn(Event) + 'static + Send + Sync>>;

        unsafe extern "C" fn on_event_cb(
            event_usr: *mut libc::c_void,
            event_type: ResponseType,
            slot_a: SlotA,
            slot_b: SlotB,
            slot_c: SlotC,
            _slot_d: SlotD,
        ) {
            let closure: DynCb = Box::from_raw(event_usr as *mut _);

            let evt = match event_type {
                TY_ERR => {
                    let err =
                        std::slice::from_raw_parts(slot_b as *const u8, slot_c);
                    Event::Error(Error::err(
                        String::from_utf8_lossy(err).to_string(),
                    ))
                }
                TY_ON_TRACE => {
                    let msg =
                        std::slice::from_raw_parts(slot_b as *const u8, slot_c);
                    let msg = String::from_utf8_lossy(msg);
                    match slot_a {
                        1 => tracing::trace!("{}", msg),
                        2 => tracing::debug!("{}", msg),
                        3 => tracing::info!("{}", msg),
                        4 => tracing::warn!("{}", msg),
                        _ => tracing::error!("{}", msg),
                    }

                    // need to forget it every time, otherwise drop will run
                    Box::into_raw(closure);
                    return;
                }
                TY_PEER_CON_ON_ICE_CANDIDATE => Event::PeerConICECandidate {
                    peer_con_id: slot_a,
                    candidate: slot_b,
                },
                TY_PEER_CON_ON_STATE_CHANGE => Event::PeerConStateChange {
                    peer_con_id: slot_a,
                    peer_con_state: slot_b,
                },
                TY_PEER_CON_ON_DATA_CHANNEL => Event::PeerConDataChan {
                    peer_con_id: slot_a,
                    data_chan_id: slot_b,
                },
                TY_DATA_CHAN_ON_CLOSE => Event::DataChanClose(slot_a),
                TY_DATA_CHAN_ON_OPEN => Event::DataChanOpen(slot_a),
                TY_DATA_CHAN_ON_MESSAGE => Event::DataChanMessage {
                    data_chan_id: slot_a,
                    buffer_id: slot_b,
                },
                oth => Event::Error(Error::err(format!(
                    "invalid event_type: {oth}",
                ))),
            };

            closure(evt);

            // need to forget it every time, otherwise drop will run
            Box::into_raw(closure);
        }

        let cb: DynCb = Box::new(Arc::new(cb));
        let cb = Box::into_raw(cb);

        let prev_usr =
            self.0.borrow_on_event()(Some(on_event_cb), cb as *mut _);

        if !prev_usr.is_null() {
            let closure: DynCb = Box::from_raw(prev_usr as *mut _);
            // *this* one we want to drop
            drop(closure);
        }
    }

    /// If the response slots are pointers to go memory, they are only valid
    /// for the duration of the callback, so make sure you know what you
    /// are doing if using this function directly.
    /// If the call slots are pointers to rust memory, go will not access them
    /// outside this call invocation.
    #[inline]
    pub unsafe fn call<Cb, R>(
        &self,
        call_type: CallType,
        slot_a: SlotA,
        slot_b: SlotB,
        slot_c: SlotC,
        slot_d: SlotD,
        cb: Cb,
    ) -> Result<R>
    where
        Cb: FnOnce(
            Result<(ResponseType, SlotA, SlotB, SlotC, SlotD)>,
        ) -> Result<R>,
    {
        let mut out = Err(Error::id("NotCalled"));
        self.call_inner(
            call_type,
            slot_a,
            slot_b,
            slot_c,
            slot_d,
            |t, a, b, c, d| {
                out = if t == TY_ERR {
                    let id = std::slice::from_raw_parts(a as *const u8, b);
                    let id = String::from_utf8_lossy(id).to_string();
                    let info = std::slice::from_raw_parts(c as *const u8, d);
                    let info = String::from_utf8_lossy(info).to_string();

                    let err = Error { id, info }.into();

                    cb(Err(err))
                } else {
                    cb(Ok((t, a, b, c, d)))
                };
            },
        );
        out
    }

    #[inline]
    unsafe fn call_inner<'lt, 'a, Cb>(
        &'lt self,
        call_type: CallType,
        slot_a: SlotA,
        slot_b: SlotB,
        slot_c: SlotC,
        slot_d: SlotD,
        cb: Cb,
    ) where
        Cb: 'a + FnOnce(ResponseType, SlotA, SlotB, SlotC, SlotD),
    {
        type DynCb<'a> =
            Box<Box<dyn FnOnce(ResponseType, SlotA, SlotB, SlotC, SlotD) + 'a>>;

        unsafe extern "C" fn call_cb(
            response_usr: *mut libc::c_void,
            response_type: ResponseType,
            slot_a: SlotA,
            slot_b: SlotB,
            slot_c: SlotC,
            slot_d: SlotD,
        ) {
            let closure: DynCb = Box::from_raw(response_usr as *mut _);

            closure(response_type, slot_a, slot_b, slot_c, slot_d);
        }

        let cb: DynCb<'a> = Box::new(Box::new(cb));
        let cb = Box::into_raw(cb);

        self.0.borrow_call()(
            call_type,
            slot_a,
            slot_b,
            slot_c,
            slot_d,
            Some(call_cb),
            cb as *mut _,
        );
    }

    /// Create a new buffer in go memory with given length,
    /// access the buffer's memory in the callback.
    #[inline]
    pub unsafe fn buffer_alloc(&self) -> Result<BufferId> {
        self.call(TY_BUFFER_ALLOC, 0, 0, 0, 0, |r| match r {
            Ok((_t, a, _b, _c, _d)) => Ok(a),
            Err(e) => Err(e),
        })
    }

    #[inline]
    pub unsafe fn buffer_free(&self, id: BufferId) {
        self.0.borrow_call()(
            TY_BUFFER_FREE,
            id,
            0,
            0,
            0,
            None,
            std::ptr::null_mut(),
        );
    }

    #[inline]
    pub unsafe fn buffer_access<Cb, R>(&self, id: BufferId, cb: Cb) -> Result<R>
    where
        Cb: FnOnce(Result<(BufferId, &mut [u8])>) -> Result<R>,
    {
        self.call(TY_BUFFER_ACCESS, id, 0, 0, 0, move |r| match r {
            Ok((_t, a, b, c, _d)) => {
                if c == 0 {
                    cb(Ok((a, &mut [])))
                } else {
                    let s = std::slice::from_raw_parts_mut(b as *mut _, c);
                    cb(Ok((a, s)))
                }
            }
            Err(e) => cb(Err(e)),
        })
    }

    #[inline]
    pub unsafe fn buffer_reserve(
        &self,
        id: BufferId,
        add: usize,
    ) -> Result<()> {
        self.call(TY_BUFFER_RESERVE, id, add, 0, 0, |r| match r {
            Ok((_t, _a, _b, _c, _d)) => Ok(()),
            Err(e) => Err(e),
        })
    }

    #[inline]
    pub unsafe fn buffer_extend(&self, id: BufferId, add: &[u8]) -> Result<()> {
        self.call(
            TY_BUFFER_EXTEND,
            id,
            add.as_ptr() as usize,
            add.len(),
            0,
            |r| match r {
                Ok((_t, _a, _b, _c, _d)) => Ok(()),
                Err(e) => Err(e),
            },
        )
    }

    #[inline]
    pub unsafe fn buffer_read<Cb, R>(
        &self,
        id: BufferId,
        len: usize,
        cb: Cb,
    ) -> Result<R>
    where
        Cb: FnOnce(Result<&mut [u8]>) -> Result<R>,
    {
        self.call(TY_BUFFER_READ, id, len, 0, 0, move |r| match r {
            Ok((_t, a, b, _c, _d)) => {
                if b == 0 {
                    cb(Ok(&mut []))
                } else {
                    let s = std::slice::from_raw_parts_mut(a as *mut _, b);
                    cb(Ok(s))
                }
            }
            Err(e) => cb(Err(e)),
        })
    }

    #[inline]
    pub unsafe fn peer_con_alloc(
        &self,
        config_buf_id: BufferId,
    ) -> Result<PeerConId> {
        self.call(TY_PEER_CON_ALLOC, config_buf_id, 0, 0, 0, |r| match r {
            Ok((_t, a, _b, _c, _d)) => Ok(a),
            Err(e) => Err(e),
        })
    }

    #[inline]
    pub unsafe fn peer_con_free(&self, id: PeerConId) {
        self.0.borrow_call()(
            TY_PEER_CON_FREE,
            id,
            0,
            0,
            0,
            None,
            std::ptr::null_mut(),
        );
    }

    #[inline]
    pub unsafe fn peer_con_create_offer(
        &self,
        id: PeerConId,
        config_buf_id: BufferId,
    ) -> Result<BufferId> {
        self.call(
            TY_PEER_CON_CREATE_OFFER,
            id,
            config_buf_id,
            0,
            0,
            |r| match r {
                Ok((_t, a, _b, _c, _d)) => Ok(a),
                Err(e) => Err(e),
            },
        )
    }

    #[inline]
    pub unsafe fn peer_con_create_answer(
        &self,
        id: PeerConId,
        config_buf_id: BufferId,
    ) -> Result<BufferId> {
        self.call(TY_PEER_CON_CREATE_ANSWER, id, config_buf_id, 0, 0, |r| {
            match r {
                Ok((_t, a, _b, _c, _d)) => Ok(a),
                Err(e) => Err(e),
            }
        })
    }

    #[inline]
    pub unsafe fn peer_con_set_local_desc(
        &self,
        id: PeerConId,
        desc_buf_id: BufferId,
    ) -> Result<()> {
        self.call(
            TY_PEER_CON_SET_LOCAL_DESC,
            id,
            desc_buf_id,
            0,
            0,
            |r| match r {
                Ok((_t, _a, _b, _c, _d)) => Ok(()),
                Err(e) => Err(e),
            },
        )
    }

    #[inline]
    pub unsafe fn peer_con_set_rem_desc(
        &self,
        id: PeerConId,
        desc_buf_id: BufferId,
    ) -> Result<()> {
        self.call(
            TY_PEER_CON_SET_REM_DESC,
            id,
            desc_buf_id,
            0,
            0,
            |r| match r {
                Ok((_t, _a, _b, _c, _d)) => Ok(()),
                Err(e) => Err(e),
            },
        )
    }

    #[inline]
    pub unsafe fn peer_con_add_ice_candidate(
        &self,
        id: PeerConId,
        ice_buf_id: BufferId,
    ) -> Result<()> {
        self.call(TY_PEER_CON_ADD_ICE_CANDIDATE, id, ice_buf_id, 0, 0, |r| {
            match r {
                Ok((_t, _a, _b, _c, _d)) => Ok(()),
                Err(e) => Err(e),
            }
        })
    }

    #[inline]
    pub unsafe fn peer_con_create_data_chan(
        &self,
        id: PeerConId,
        config_buf_id: BufferId,
    ) -> Result<DataChanId> {
        self.call(TY_PEER_CON_CREATE_DATA_CHAN, id, config_buf_id, 0, 0, |r| {
            match r {
                Ok((_t, a, _b, _c, _d)) => Ok(a),
                Err(e) => Err(e),
            }
        })
    }

    #[inline]
    pub unsafe fn data_chan_free(&self, id: DataChanId) {
        self.0.borrow_call()(
            TY_DATA_CHAN_FREE,
            id,
            0,
            0,
            0,
            None,
            std::ptr::null_mut(),
        );
    }

    #[inline]
    pub unsafe fn data_chan_ready_state(
        &self,
        id: DataChanId,
    ) -> Result<usize> {
        self.call(TY_DATA_CHAN_READY_STATE, id, 0, 0, 0, |r| match r {
            Ok((_t, a, _b, _c, _d)) => Ok(a),
            Err(e) => Err(e),
        })
    }

    #[inline]
    pub unsafe fn data_chan_label(&self, id: DataChanId) -> Result<BufferId> {
        self.call(TY_DATA_CHAN_LABEL, id, 0, 0, 0, |r| match r {
            Ok((_t, a, _b, _c, _d)) => Ok(a),
            Err(e) => Err(e),
        })
    }

    #[inline]
    pub unsafe fn data_chan_send(
        &self,
        id: DataChanId,
        buffer_id: BufferId,
    ) -> Result<()> {
        self.call(TY_DATA_CHAN_SEND, id, buffer_id, 0, 0, |r| match r {
            Ok((_t, _a, _b, _c, _d)) => Ok(()),
            Err(e) => Err(e),
        })
    }
}

/// The main entrypoint for working with this ffi binding crate.
pub static API: Lazy<Api> = Lazy::new(Api::priv_new);
