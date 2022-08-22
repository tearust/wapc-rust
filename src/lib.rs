#![doc(html_logo_url = "https://avatars0.githubusercontent.com/u/54989751?s=200&v=4")]

//! # wapc
//!
//! The `wapc` crate provides a WebAssembly host runtime that conforms to an RPC mechanism
//! called **waPC**. waPC is designed specifically to prevent either side of the call from having
//! to know anything about _how_ or _when_ memory is allocated or freed. The interface may at first appear more
//! "chatty" than other protocols, but the cleanliness, ease of use, and simplified developer experience
//! is worth the few extra nanoseconds of latency.
//!
//! To use `wapc`, first you'll need a waPC-compliant WebAssembly module (referred to as the _guest_) to load
//! and interpret. You can find a number of these samples available in the GitHub repository,
//! and anything compiled with the [wascc](https://github.com/wascc) actor SDK can also be invoked
//! via waPC as it is 100% waPC compliant.
//!
//! To make function calls, first set your `host_callback` function, a function invoked by the _guest_.
//! Then execute `call` on the `WapcHost` instance.
//! # Example
//! ```
//! extern crate wapc;
//! use wapc::prelude::*;
//! use wapc::Result;
//!
//! # fn load_file() -> Vec<u8> {
//! #    include_bytes!("../.assets/hello.wasm").to_vec()
//! # }
//! # fn load_wasi_file() -> Vec<u8> {
//! #    include_bytes!("../.assets/hello_wasi.wasm").to_vec()
//! # }
//! pub fn main() -> Result<()> {
//!     let module_bytes = load_file();
//!     let mut host = WapcHost::new(|id: u64, bd: &str, ns: &str, op: &str, payload: &[u8]| {
//!         println!("Guest {} invoked '{}->{}:{}' with payload of {} bytes", id, bd, ns, op, payload.len());
//!         Ok(vec![])
//!     }, &module_bytes, None)?;
//!
//!     let res = host.call("wapc:sample!Hello", b"this is a test")?;
//!     assert_eq!(res, b"hello world!");
//!
//!     Ok(())
//! }
//! ```
//!
//! # Notes
//! waPC is _reactive_. Guest modules cannot initiate host calls without first handling a call
//! initiated by the host. waPC will not automatically invoke any start functions--that decision
//! is up to the waPC library consumer. Guest modules can synchronously make as many host calls
//! as they like, but keep in mind that if a host call takes too long or fails, it'll cause the original
//! guest call to also fail.
//!
//! In summary, keep `host_callback` functions fast and resilient, and do not spawn new threads
//! within `host_callback` unless you must (and can synchronize memory access) because waPC
//! assumes a single-threaded execution environment. The `host_callback` function intentionally
//! has no references to the WebAssembly module bytes or the running instance.

#![feature(generic_associated_types)]
#![feature(min_specialization)]

#[macro_use]
extern crate log;

mod callbacks;
pub mod error;
mod modreg;
pub mod prelude;

/// A result type for errors that occur within the wapc library
pub use error::Result;
use error::{Error, GuestCallFailure, WasmMisc};

use crate::callbacks::ModuleState;
use crate::modreg::ModuleRegistry;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use wasmtime::Func;
use wasmtime::Instance;
use wasmtime::*;

static GLOBAL_MODULE_COUNT: AtomicU64 = AtomicU64::new(1);

const HOST_NAMESPACE: &str = "wapc";

// -- Functions called by guest, exported by host
const HOST_CONSOLE_LOG: &str = "__console_log";
const HOST_CALL: &str = "__host_call";
const GUEST_REQUEST_FN: &str = "__guest_request";
const HOST_RESPONSE_FN: &str = "__host_response";
const HOST_RESPONSE_LEN_FN: &str = "__host_response_len";
const GUEST_RESPONSE_FN: &str = "__guest_response";
const GUEST_ERROR_FN: &str = "__guest_error";
const HOST_ERROR_FN: &str = "__host_error";
const HOST_ERROR_LEN_FN: &str = "__host_error_len";

// -- Functions called by host, exported by guest
const GUEST_CALL: &str = "__guest_call";

type HostCallback = dyn Fn(u64, &str, &str, &str, &[u8]) -> std::result::Result<Vec<u8>, Error>
	+ Sync
	+ Send
	+ 'static;

type LogCallback = dyn Fn(u64, &str) -> std::result::Result<(), Error> + Sync + Send + 'static;

#[derive(Debug, Clone)]
pub struct Invocation {
	operation: String,
	msg: Vec<u8>,
}

impl Invocation {
	fn new(op: &str, msg: Vec<u8>) -> Invocation {
		Invocation {
			operation: op.to_string(),
			msg,
		}
	}
}

/// Stores the parameters required to create a WASI instance
#[derive(Debug, Default)]
pub struct WasiParams {
	#[allow(dead_code)]
	argv: Vec<String>,
	map_dirs: Vec<(String, String)>,
	env_vars: Vec<(String, String)>,
	preopened_dirs: Vec<String>,
}

impl WasiParams {
	pub fn new(
		argv: Vec<String>,
		map_dirs: Vec<(String, String)>,
		env_vars: Vec<(String, String)>,
		preopened_dirs: Vec<String>,
	) -> Self {
		WasiParams {
			argv,
			map_dirs,
			preopened_dirs,
			env_vars,
		}
	}
}

/// A WebAssembly host runtime for waPC-compliant WebAssembly modules
///
/// Use an instance of this struct to provide a means of invoking procedure calls by
/// specifying an operation name and a set of bytes representing the opaque operation payload.
/// `WapcHost` makes no assumptions about the contents or format of either the payload or the
/// operation name.
pub struct WapcHost {
	state: Arc<RefCell<ModuleState>>,
	store: Rc<RefCell<Option<Store<ModuleRegistry>>>>,
	instance: Rc<RefCell<Option<Instance>>>,
	wasidata: Option<WasiParams>,
	guest_call_fn: Func,
}

impl WapcHost {
	/// Creates a new instance of a waPC-compliant WebAssembly host runtime.
	pub fn new(
		host_callback: impl Fn(u64, &str, &str, &str, &[u8]) -> Result<Vec<u8>> + 'static + Sync + Send,
		buf: &[u8],
		wasi: Option<WasiParams>,
	) -> Result<Self> {
		let id = GLOBAL_MODULE_COUNT.fetch_add(1, Ordering::SeqCst);
		let state = Arc::new(RefCell::new(ModuleState::new(id, Box::new(host_callback))));
		let (mut store, instance) = WapcHost::instance_from_buffer(buf, &wasi, state.clone())?;
		let gc = guest_call_fn(&mut store, &instance)?;
		let mh = WapcHost {
			state,
			store: Rc::new(RefCell::new(Some(store))),
			instance: Rc::new(RefCell::new(Some(instance))),
			wasidata: wasi,
			guest_call_fn: gc,
		};

		mh.initialize()?;

		Ok(mh)
	}

	/// Creates a new instance of a waPC-compliant WebAssembly host runtime with a callback handler
	/// for logging
	pub fn new_with_logger(
		host_callback: impl Fn(u64, &str, &str, &str, &[u8]) -> Result<Vec<u8>> + 'static + Sync + Send,
		buf: &[u8],
		logger: impl Fn(u64, &str) -> Result<()> + Sync + Send + 'static,
		wasi: Option<WasiParams>,
	) -> Result<Self> {
		let id = GLOBAL_MODULE_COUNT.fetch_add(1, Ordering::SeqCst);
		let state = Arc::new(RefCell::new(ModuleState::new_with_logger(
			id,
			Box::new(host_callback),
			Box::new(logger),
		)));
		let (mut store, instance) = WapcHost::instance_from_buffer(buf, &wasi, state.clone())?;
		let gc = guest_call_fn(&mut store, &instance)?;
		let mh = WapcHost {
			state,
			store: Rc::new(RefCell::new(Some(store))),
			instance: Rc::new(RefCell::new(Some(instance))),
			wasidata: wasi,
			guest_call_fn: gc,
		};

		mh.initialize()?;

		Ok(mh)
	}

	/// Returns a reference to the unique identifier of this module. If a parent process
	/// has instantiated multiple `WapcHost`s, then the single static host call function
	/// may be used to differentiate between modules.
	pub fn id(&self) -> u64 {
		self.state.borrow().id
	}

	/// Invokes the `__guest_call` function within the guest module as per the waPC specification.
	/// Provide an operation name and an opaque payload of bytes and the function returns a `Result`
	/// containing either an error or an opaque reply of bytes.    
	///
	/// It is worth noting that the _first_ time `call` is invoked, the WebAssembly module
	/// will be JIT-compiled. This can take up to a few seconds on debug .wasm files, but
	/// all subsequent calls will be "hot" and run at near-native speeds.    
	pub fn call(&mut self, op: &str, payload: &[u8]) -> Result<Vec<u8>> {
		let inv = Invocation::new(op, payload.to_vec());

		{
			let mut state = self.state.borrow_mut();
			state.guest_response = None;
			state.guest_request = Some((inv).clone());
			state.guest_error = None;
		}

		let mut store_mut = self.store.borrow_mut();
		let mut store_ctx = store_mut
			.as_mut()
			.ok_or(WasmMisc("failed to get store".to_owned()))?
			.as_context_mut();
		let callresult = self
			.guest_call_fn
			.typed::<(i32, i32), i32, _>(&store_ctx)
			.map_err(|e| WasmMisc(format!("convert typed guest call fn failed: {}", e).into()))?
			.call(
				&mut store_ctx,
				(inv.operation.len() as i32, inv.msg.len() as i32),
			)
			.map_err(|e| WasmMisc(format!("guest call failed: {}", e)))?;

		if callresult == 0 {
			// invocation failed
			match self.state.borrow().guest_error {
				Some(ref s) => Err(GuestCallFailure(s.clone()).into()),
				None => {
					Err(GuestCallFailure("No error message set for call failure".into()).into())
				}
			}
		} else {
			// invocation succeeded
			match self.state.borrow().guest_response {
				Some(ref e) => Ok(e.clone()),
				None => match self.state.borrow().guest_error {
					Some(ref s) => Err(GuestCallFailure(s.clone()).into()),
					None => Err(GuestCallFailure(
						"No error message OR response set for call success".into(),
					)
					.into()),
				},
			}
		}
	}

	/// Performs a live "hot swap" of the WebAssembly module. Since execution is assumed to be
	/// single-threaded within the environment of the `WapcHost`, this will not cause any pending function
	/// calls to be lost. This will replace the currently executing WebAssembly module with the new
	/// bytes.
	///
	/// **Note**: you will lose all JITted functions for this module, so the first `call` after a
	/// hot swap will be "cold" and take longer than regular calls. There are an enormous number of
	/// ways in which a hot swap could go horribly wrong, so please ensure you have the proper guards
	/// in place before invoking it. Libraries that build upon this one can (and likely should) implement
	/// some form of security to protect against malicious swaps.
	///
	/// If you perform a hot swap of a WASI module, you cannot alter the parameters used to create the WASI module
	/// like the environment variables, mapped directories, pre-opened files, etc. Not abiding by this could lead
	/// to privilege escalation attacks or non-deterministic behavior after the swap.
	pub fn replace_module(&self, module: &[u8]) -> Result<()> {
		info!(
			"HOT SWAP - Replacing existing WebAssembly module with new buffer, {} bytes",
			module.len()
		);
		let state = self.state.clone();
		let (store, new_instance) = WapcHost::instance_from_buffer(module, &self.wasidata, state)?;
		self.instance.borrow_mut().replace(new_instance);
		self.store.borrow_mut().replace(store);

		self.initialize()
	}

	fn instance_from_buffer(
		buf: &[u8],
		wasi: &Option<WasiParams>,
		state: Arc<RefCell<ModuleState>>,
	) -> Result<(Store<ModuleRegistry>, Instance)> {
		let engine = Engine::default();

		let d = WasiParams::default();
		let wasi = match wasi {
			Some(w) => w,
			None => &d,
		};

		// Make wasi available by default.
		let preopen_dirs =
			modreg::compute_preopen_dirs(&wasi.preopened_dirs, &wasi.map_dirs).unwrap();
		let argv = vec![]; // TODO: add support for argv (if applicable)
		let module_registry =
			ModuleRegistry::new(&preopen_dirs, &argv, &wasi.env_vars, state).unwrap();

		let mut store = Store::new(&engine, module_registry);
		let module = Module::new(&engine, buf).unwrap();

		let mut linker = Linker::new(&engine);
		wasmtime_wasi::add_to_linker(&mut linker, |s: &mut ModuleRegistry| &mut s.ctx)
			.map_err(|e| WasmMisc(format!("wasmtime wasi add to linker failed: {}", e)))?;

		arrange_imports(&mut linker)?;

		let instance = linker
			.instantiate(&mut store, &module)
			.map_err(|e| WasmMisc(format!("wasmtime instantiate failed: {}", e)))?;
		Ok((store, instance))
	}

	fn initialize(&self) -> Result<()> {
		let mut store_mut = self.store.borrow_mut();
		let mut store_ctx = store_mut
			.as_mut()
			.ok_or(WasmMisc("failed to get store".to_owned()))?
			.as_context_mut();
		if let Some(ext) = self
			.instance
			.borrow()
			.as_ref()
			.unwrap()
			.get_export(&mut store_ctx, "_start")
		{
			ext.into_func()
				.unwrap()
				.call(&mut store_ctx, &[], &mut [])
				.map(|_| ())
				.map_err(|_err| GuestCallFailure("Error invoking _start function!".into()).into())
		} else {
			Ok(())
		}
	}
}

// Called once, then the result is cached. This returns a `Func` that corresponds
// to the `__guest_call` export
fn guest_call_fn(store: &mut Store<ModuleRegistry>, instance: &Instance) -> Result<Func> {
	if let Some(ext) = instance.get_export(store, GUEST_CALL) {
		Ok(ext.into_func().unwrap().clone())
	} else {
		Err(GuestCallFailure("Guest module did not export __guest_call function!".into()).into())
	}
}

/// wasmtime requires that the list of callbacks be "zippable" with the list
/// of module imports. In order to ensure that both lists are in the same
/// order, we have to loop through the module imports and instantiate the
/// corresponding callback. We **cannot** rely on a predictable import order
/// in the wasm module
fn arrange_imports(linker: &mut Linker<ModuleRegistry>) -> Result<()> {
	let export_funcs = [
		HOST_CONSOLE_LOG,
		HOST_CALL,
		GUEST_REQUEST_FN,
		HOST_RESPONSE_FN,
		HOST_RESPONSE_LEN_FN,
		GUEST_RESPONSE_FN,
		GUEST_ERROR_FN,
		HOST_ERROR_FN,
		HOST_ERROR_LEN_FN,
	];

	for name in export_funcs {
		callback_for_import(name, linker)?;
	}

	Ok(())
}

fn callback_for_import(import: &str, linker: &mut Linker<ModuleRegistry>) -> Result<()> {
	match import {
		HOST_CONSOLE_LOG => callbacks::console_log_func(linker),
		HOST_CALL => callbacks::host_call_func(linker),
		GUEST_REQUEST_FN => callbacks::guest_request_func(linker),
		HOST_RESPONSE_FN => callbacks::host_response_func(linker),
		HOST_RESPONSE_LEN_FN => callbacks::host_response_len_func(linker),
		GUEST_RESPONSE_FN => callbacks::guest_response_func(linker),
		GUEST_ERROR_FN => callbacks::guest_error_func(linker),
		HOST_ERROR_FN => callbacks::host_error_func(linker),
		HOST_ERROR_LEN_FN => callbacks::host_error_len_func(linker),
		_ => unreachable!(),
	}
}
