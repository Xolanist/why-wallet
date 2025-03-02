// Copyright 2020 The MWC Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! lower-level wallet functions which build upon core::libtx to perform wallet
//! operations

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

/// Some crypto releted utils.
pub mod crypto;
/// Key derivation that come froom why713. Expected that they will be used for all transports
pub mod hasher;
/// Proff messages
pub mod message;
/// Addresses
pub mod proofaddress;
/// Proofs that come froom why713. Expected that they will be used for all transports
pub mod tx_proof;

///
pub mod base58;
