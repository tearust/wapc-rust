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

use tea_codec::error::{
	new_common_error_code, new_wascc_error_code, CommonCode, TeaError, WasccCode,
};

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
			ErrorKind::NoSuchFunction(s) => {
				new_wascc_error_code(WasccCode::NoSuchFunction).to_error_code(Some(s), None)
			}
			ErrorKind::IO(e) => new_common_error_code(CommonCode::StdIoError)
				.to_error_code(Some(format!("{:?}", e)), None),
			ErrorKind::WasmMisc(s) => {
				new_wascc_error_code(WasccCode::WasmMisc).to_error_code(Some(s), None)
			}
			ErrorKind::HostCallFailure(inner) => {
				new_wascc_error_code(WasccCode::HostCallFailure).error_from_nested(inner)
			}
			ErrorKind::GuestCallFailure(inner) => {
				new_wascc_error_code(WasccCode::GuestCallFailure).error_from_nested(inner)
			}
		}
	}
}

impl From<std::io::Error> for WapcError {
	fn from(source: std::io::Error) -> WapcError {
		WapcError(Box::new(ErrorKind::IO(source)))
	}
}
