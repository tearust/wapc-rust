//! Taken from the wasmtime CLI

use crate::{
	callbacks::ModuleState,
	error::{Result, WasmMisc},
};
use std::{
	cell::RefCell,
	ffi::OsStr,
	fs::File,
	path::{Component, PathBuf},
	sync::Arc,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder};

pub struct ModuleRegistry {
	pub ctx: WasiCtx,
	pub state: Arc<RefCell<ModuleState>>,
}

impl ModuleRegistry {
	pub fn new(
		_preopen_dirs: &[(String, File)],
		argv: &[String],
		vars: &[(String, String)],
		state: Arc<RefCell<ModuleState>>,
	) -> Result<ModuleRegistry> {
		let builder = WasiCtxBuilder::new()
			.args(argv)
			.map_err(|e| WasmMisc(format!("wasi ctx build args {:?} error: {}", argv, e)))?
			.envs(vars)
			.map_err(|e| WasmMisc(format!("wasi ctx build envs {:?} error: {}", vars, e)))?;
		// todo deal with preopen_dirs

		Ok(ModuleRegistry {
			state,
			ctx: builder.build(),
		})
	}
}

pub(crate) fn compute_preopen_dirs(
	_dirs: &Vec<String>,
	_map_dirs: &Vec<(String, String)>,
) -> Result<Vec<(String, File)>> {
	// todo complete me
	Ok(vec![])
	// let mut preopen_dirs = Vec::new();

	// for dir in dirs.iter() {
	// 	preopen_dirs.push((
	// 		dir.clone(),
	// 		preopen_dir(dir)
	// 			.with_context(|| format!("failed to open directory '{}'", dir))
	// 			.unwrap(), // TODO: get rid of unwrap
	// 	));
	// }

	// for (guest, host) in map_dirs.iter() {
	// 	preopen_dirs.push((
	// 		guest.clone(),
	// 		preopen_dir(host)
	// 			.with_context(|| format!("failed to open directory '{}'", host))
	// 			.unwrap(), // TODO: get rid of unwrap
	// 	));
	// }

	// Ok(preopen_dirs)
}

#[allow(dead_code)]
pub(crate) fn compute_argv(module: PathBuf, module_args: Vec<String>) -> Vec<String> {
	let mut result = Vec::new();

	// Add argv[0], which is the program name. Only include the base name of the
	// main wasm module, to avoid leaking path information.
	result.push(
		module
			.components()
			.next_back()
			.map(Component::as_os_str)
			.and_then(OsStr::to_str)
			.unwrap_or("")
			.to_owned(),
	);

	// Add the remaining arguments.
	for arg in module_args.iter() {
		result.push(arg.clone());
	}

	result
}
