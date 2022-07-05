use crate::{
	errors, modreg::ModuleRegistry, HostCallback, Invocation, LogCallback, WapcResult,
	GUEST_ERROR_FN, GUEST_REQUEST_FN, GUEST_RESPONSE_FN, HOST_CALL, HOST_CONSOLE_LOG,
	HOST_ERROR_FN, HOST_ERROR_LEN_FN, HOST_NAMESPACE, HOST_RESPONSE_FN, HOST_RESPONSE_LEN_FN,
};
use std::convert::TryInto;
use tea_codec::{deserialize, error::TeaError, serialize};
use wasmtime::{
	AsContext, AsContextMut, Caller, FuncType, Linker, Memory, StoreContext, StoreContextMut, Trap,
	Val, ValType,
};

#[derive(Default)]
pub struct ModuleState {
	pub guest_request: Option<Invocation>,
	pub guest_response: Option<Vec<u8>>,
	pub host_response: Option<Vec<u8>>,
	pub guest_error: Option<TeaError>,
	pub host_error: Option<TeaError>,
	pub host_callback: Option<Box<HostCallback>>,
	pub log_callback: Option<Box<LogCallback>>,
	pub id: u64,
}

impl ModuleState {
	pub fn new(id: u64, host_callback: Box<HostCallback>) -> Self {
		ModuleState {
			id,
			host_callback: Some(host_callback),
			log_callback: None,
			..ModuleState::default()
		}
	}

	pub fn new_with_logger(
		id: u64,
		host_callback: Box<HostCallback>,
		log_callback: Box<LogCallback>,
	) -> Self {
		ModuleState {
			id,
			host_callback: Some(host_callback),
			log_callback: Some(log_callback),
			..ModuleState::default()
		}
	}
}

pub(crate) fn guest_request_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			GUEST_REQUEST_FN,
			FuncType::new([ValType::I32, ValType::I32], []),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let ptr = params[1].i32();
				let op_ptr = params[0].i32();

				let state = caller.data().state.clone();
				let invocation = &state.borrow().guest_request;
				let memory = get_caller_memory(&mut caller).unwrap();
				if let Some(inv) = invocation {
					write_bytes_to_memory(&memory, caller.as_context_mut(), ptr.unwrap(), &inv.msg);
					write_bytes_to_memory(
						&memory,
						caller.as_context_mut(),
						op_ptr.unwrap(),
						&inv.operation.as_bytes(),
					);
				}
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap guest request func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn console_log_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_CONSOLE_LOG,
			FuncType::new([ValType::I32, ValType::I32], []),
			move |mut caller: Caller<ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let ptr = params[0].i32();
				let len = params[1].i32();
				let memory = get_caller_memory(&mut caller).unwrap();
				let vec =
					get_vec_from_memory(&memory, caller.as_context(), ptr.unwrap(), len.unwrap());

				let id = caller.data().state.borrow().id;
				let msg = std::str::from_utf8(&vec).unwrap();

				match caller.data().state.borrow().log_callback {
					Some(ref f) => {
						f(id, msg).unwrap();
					}
					None => {
						info!("[Guest {}]: {}", id, msg);
					}
				}
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap console log func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn host_call_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_CALL,
			FuncType::new(
				[
					ValType::I32,
					ValType::I32,
					ValType::I32,
					ValType::I32,
					ValType::I32,
					ValType::I32,
					ValType::I32,
					ValType::I32,
				],
				[ValType::I32],
			),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], results: &mut [Val]| {
				let id = {
					let mut state = caller.data().state.borrow_mut();
					state.host_response = None;
					state.host_error = None;
					state.id
				};
				let memory = get_caller_memory(&mut caller).unwrap();

				let bd_ptr = params[0].i32();
				let bd_len = params[1].i32();
				let ns_ptr = params[2].i32();
				let ns_len = params[3].i32();
				let op_ptr = params[4].i32();
				let op_len = params[5].i32();
				let ptr = params[6].i32();
				let len = params[7].i32();

				let vec =
					get_vec_from_memory(&memory, caller.as_context(), ptr.unwrap(), len.unwrap());
				let bd_vec = get_vec_from_memory(
					&memory,
					caller.as_context(),
					bd_ptr.unwrap(),
					bd_len.unwrap(),
				);
				let bd = std::str::from_utf8(&bd_vec).unwrap();
				let ns_vec = get_vec_from_memory(
					&memory,
					caller.as_context(),
					ns_ptr.unwrap(),
					ns_len.unwrap(),
				);
				let ns = std::str::from_utf8(&ns_vec).unwrap();
				let op_vec = get_vec_from_memory(
					&memory,
					caller.as_context(),
					op_ptr.unwrap(),
					op_len.unwrap(),
				);
				let op = std::str::from_utf8(&op_vec).unwrap();
				trace!("Guest {} invoking host operation {}", id, op);
				let result = {
					match caller.data().state.borrow().host_callback {
						Some(ref f) => f(id, bd, ns, op, &vec),
						None => Err(TeaError::CommonError(
							"missing host callback function".into(),
						)),
					}
				};
				results[0] = Val::I32(match result {
					Ok(invresp) => {
						caller.data().state.borrow_mut().host_response = Some(invresp);
						1
					}
					Err(e) => {
						caller.data().state.borrow_mut().host_error = Some(e);
						0
					}
				});

				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap host call func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn host_response_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_RESPONSE_FN,
			FuncType::new([ValType::I32], []),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let store = caller.data().state.clone();
				if let Some(ref e) = store.borrow().host_response.clone() {
					let memory = get_caller_memory(&mut caller).unwrap();
					let ptr = params[0].i32();
					write_bytes_to_memory(&memory, caller.as_context_mut(), ptr.unwrap(), &e);
				}
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap host response func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn host_response_len_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_RESPONSE_LEN_FN,
			FuncType::new([], [ValType::I32]),
			move |caller: Caller<'_, ModuleRegistry>, _params: &[Val], results: &mut [Val]| {
				results[0] = Val::I32(match caller.data().state.borrow().host_response {
					Some(ref r) => r.len() as _,
					None => 0,
				});
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap host response len func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn guest_response_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			GUEST_RESPONSE_FN,
			FuncType::new([ValType::I32, ValType::I32], []),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let ptr = params[0].i32();
				let len = params[1].i32();
				let memory = get_caller_memory(&mut caller).unwrap();
				let vec =
					get_vec_from_memory(&memory, caller.as_context(), ptr.unwrap(), len.unwrap());
				caller.data().state.borrow_mut().guest_response = Some(vec);
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap guest response func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn guest_error_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			GUEST_ERROR_FN,
			FuncType::new([ValType::I32, ValType::I32], []),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let memory = get_caller_memory(&mut caller).unwrap();
				let ptr = params[0].i32();
				let len = params[1].i32();

				let vec =
					get_vec_from_memory(&memory, caller.as_context(), ptr.unwrap(), len.unwrap());
				caller.data().state.borrow_mut().guest_error =
					Some(deserialize(&vec).map_err(|e| Trap::new(format!("{:?}", e)))?);

				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap guest error func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn host_error_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_ERROR_FN,
			FuncType::new([ValType::I32], []),
			move |mut caller: Caller<'_, ModuleRegistry>, params: &[Val], _results: &mut [Val]| {
				let state = caller.data().state.clone();
				if let Some(e) = state.borrow().host_error.clone() {
					let ptr = params[0].i32();
					let memory = get_caller_memory(&mut caller).unwrap();
					let buf = serialize(&e)
						.map_err(|e| Trap::new(format!("serialize host error failed: {:?}", e)))?;
					write_bytes_to_memory(
						&memory,
						caller.as_context_mut(),
						ptr.unwrap(),
						buf.as_slice(),
					);
				}
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap host error func failed: {}",
				e
			)))
		})?;
	Ok(())
}

pub(crate) fn host_error_len_func(linker: &mut Linker<ModuleRegistry>) -> WapcResult<()> {
	let callback_type = FuncType::new([], [ValType::I32]);
	linker
		.func_new(
			HOST_NAMESPACE,
			HOST_ERROR_LEN_FN,
			callback_type,
			move |caller: Caller<'_, ModuleRegistry>, _params: &[Val], results: &mut [Val]| {
				results[0] = Val::I32(match caller.data().state.borrow().host_error {
					Some(ref e) => {
						let buf = serialize(e).map_err(|e| {
							Trap::new(format!("serialize host error failed: {:?}", e))
						})?;
						buf.len().try_into().map_err(|e| {
							Trap::new(format!("try convert host error len failed: {:?}", e))
						})?
					}
					None => 0,
				});
				Ok(())
			},
		)
		.map_err(|e| {
			errors::new(errors::ErrorKind::WasmMisc(format!(
				"wrap host error len func failed: {}",
				e
			)))
		})?;
	Ok(())
}

fn get_caller_memory(caller: &mut Caller<'_, ModuleRegistry>) -> Result<Memory, anyhow::Error> {
	let memory = caller
		.get_export("memory")
		.map(|e| e.into_memory().unwrap());
	Ok(memory.unwrap())
}

fn get_vec_from_memory(
	mem: &Memory,
	store: StoreContext<ModuleRegistry>,
	ptr: i32,
	len: i32,
) -> Vec<u8> {
	let data = mem.data(store);
	data[ptr as usize..(ptr + len) as usize]
		.iter()
		.copied()
		.collect()
}

fn write_bytes_to_memory(
	memory: &Memory,
	store: StoreContextMut<ModuleRegistry>,
	ptr: i32,
	slice: &[u8],
) {
	let data = memory.data_mut(store);
	for idx in 0..slice.len() {
		data[idx + ptr as usize] = slice[idx];
	}
}
