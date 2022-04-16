// Copyright 2015-2019 Capital One Services, LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Library-specific error types and utility functions

use std::error::Error as StdError;
use std::fmt;
use tea_codec::error::code::common::{new_common_error_code, STD_IO_ERROR};
use tea_codec::error::code::wascc::{GUEST_CALL_FAILURE, HOST_CALL_FAILURE, new_wascc_error_code, NO_SUCH_FUNCTION, WASM_MISC};
use tea_codec::error::TeaError;

#[derive(Debug)]
pub struct WapcError(Box<ErrorKind>);

pub fn new(kind: ErrorKind) -> WapcError {
	WapcError(Box::new(kind))
}

#[derive(Debug)]
pub enum ErrorKind {
	NoSuchFunction(String),
	IO(std::io::Error),
	WasmMisc(String),
	HostCallFailure(TeaError),
	GuestCallFailure(TeaError),
}

impl WapcError {
	pub fn kind(&self) -> &ErrorKind {
		&self.0
	}

	pub fn into_kind(self) -> ErrorKind {
		*self.0
	}
}

impl Into<TeaError> for WapcError {
	fn into(self) -> TeaError {
		match *self.0 {
			ErrorKind::NoSuchFunction(s) => new_wascc_error_code(NO_SUCH_FUNCTION).to_error_code(Some(s), None),
			ErrorKind::IO(e) => new_common_error_code(STD_IO_ERROR).to_error_code(Some(format!("{:?}", e)), None ),
			ErrorKind::WasmMisc(s) => new_wascc_error_code(WASM_MISC).to_error_code(Some(s), None),
			ErrorKind::HostCallFailure(inner) => new_wascc_error_code(HOST_CALL_FAILURE).error_from_nested(inner),
			ErrorKind::GuestCallFailure(inner) => new_wascc_error_code(GUEST_CALL_FAILURE).error_from_nested(inner),
		}
	}
}

impl StdError for WapcError {
	fn description(&self) -> &str {
		match *self.0 {
			ErrorKind::NoSuchFunction(_) => "No such function in Wasm module",
			ErrorKind::IO(_) => "I/O error",
			ErrorKind::WasmMisc(_) => "WebAssembly failure",
			ErrorKind::HostCallFailure(_) => "Error occurred during host call",
			ErrorKind::GuestCallFailure(_) => "Guest call failure",
		}
	}

	fn cause(&self) -> Option<&dyn StdError> {
		match *self.0 {
			ErrorKind::NoSuchFunction(_) => None,
			ErrorKind::IO(ref err) => Some(err),
			ErrorKind::WasmMisc(_) => None,
			ErrorKind::HostCallFailure(_) => None,
			ErrorKind::GuestCallFailure(_) => None,
		}
	}
}

impl fmt::Display for WapcError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self.0 {
			ErrorKind::NoSuchFunction(ref fname) => {
				write!(f, "No such function in Wasm module: {}", fname)
			}
			ErrorKind::IO(ref err) => write!(f, "I/O error: {}", err),
			ErrorKind::WasmMisc(ref err) => write!(f, "WebAssembly error: {}", err),
			ErrorKind::HostCallFailure(ref err) => {
				write!(f, "Error occurred during host call: {:?}", err)
			}
			ErrorKind::GuestCallFailure(ref reason) => write!(f, "Guest call failure: {:?}", reason),
		}
	}
}

impl From<std::io::Error> for WapcError {
	fn from(source: std::io::Error) -> WapcError {
		WapcError(Box::new(ErrorKind::IO(source)))
	}
}
