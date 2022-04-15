extern crate wapc;
use std::fs::File;
use std::io::prelude::*;
use tea_codec::error::TeaError;
use wapc::prelude::*;
use wapc::WapcResult;

fn load_file() -> Vec<u8> {
    let mut f = File::open(".assets/hello_zig.wasm").unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    buf
}

pub fn main() -> WapcResult<()> {
    env_logger::init();
    let module_bytes = load_file();
    let mut host = WapcHost::new(host_callback, &module_bytes, None)?;

    println!("Calling guest (wasm) function written in Zig");
    let res = host.call("hello", b"this is a test")?;
    println!("Result - {}", ::std::str::from_utf8(&res).unwrap());

    Ok(())
}

fn host_callback(
    id: u64,
    bd: &str,
    ns: &str,
    op: &str,
    payload: &[u8],
) -> Result<Vec<u8>, TeaError> {
    println!(
        "Guest {} invoked '{}->{}:{}' with payload of {}",
        id,
        bd,
        ns,
        op,
        ::std::str::from_utf8(payload).unwrap()
    );
    Ok(vec![])
}
