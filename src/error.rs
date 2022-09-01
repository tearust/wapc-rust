use tea_codec::define_scope;

define_scope! {
	Wapc {
		NoSuchFunction as v => NoSuchFunction, format!("No such function in Wasm module: {}", v.0), @Debug;
		WasmMisc as v => WasmMisc, "WebAssembly failure", @Debug;
		HostCallFailure as v => HostCallFailure, format!("Error occurred during host call: {}", v.0), @Debug, [&v.0];
		GuestCallFailure as v => GuestCallFailure, format!("Guest call failure: {}", v.0), @Debug, [&v.0];
	}
}

#[derive(Debug)]
pub struct NoSuchFunction(pub String);

#[derive(Debug)]
pub struct WasmMisc(pub String);

#[derive(Debug)]
pub struct HostCallFailure(pub Error);

#[derive(Debug)]
pub struct GuestCallFailure(pub Error);
