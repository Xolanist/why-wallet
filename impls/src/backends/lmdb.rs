// Copyright 2019 The Grin Developers
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

use std::cell::RefCell;
use std::{fs, path};

// for writing stored transaction files
use std::fs::File;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::Path;

use crate::blake2::blake2b::{Blake2b, Blake2bResult};

use crate::keychain::{ChildNumber, ExtKeychain, Identifier, Keychain, SwitchCommitmentType};
use crate::store::{self, option_to_not_found, to_key, to_key_u64, u64_to_key};

use crate::core::core::Transaction;
use crate::core::ser;
use crate::libwallet::{
	swap::ethereum::EthereumWallet, AcctPathMapping, Context, Error, ErrorKind, NodeClient,
	OutputData, ScannedBlockInfo, TxLogEntry, TxProof, WalletBackend, WalletOutputBatch,
};
use crate::util::secp::constants::SECRET_KEY_SIZE;
use crate::util::secp::key::SecretKey;
use crate::util::{self, secp};

use grin_wallet_libwallet::IntegrityContext;
use rand::rngs::mock::StepRng;
use rand::thread_rng;

pub const DB_DIR: &str = "db";
pub const TX_SAVE_DIR: &str = "saved_txs";

const OUTPUT_PREFIX: u8 = b'o';
const DERIV_PREFIX: u8 = b'd';
const CONFIRMED_HEIGHT_PREFIX: u8 = b'c';
const PRIVATE_TX_CONTEXT_PREFIX: u8 = b'p';
const TX_LOG_ENTRY_PREFIX: u8 = b't';
const TX_LOG_ID_PREFIX: u8 = b'i';
const ACCOUNT_PATH_MAPPING_PREFIX: u8 = b'a';
const LAST_SCANNED_BLOCK: u8 = b'm'; // pre v3.0 was l
const LAST_WORKING_NODE_INDEX: u8 = b'n';
const INTEGRITY_CONTEXT_PREFIX: u8 = b'g';

/// test to see if database files exist in the current directory. If so,
/// use a DB backend for all operations
pub fn wallet_db_exists(data_file_dir: &str) -> bool {
	let db_path = path::Path::new(data_file_dir).join(DB_DIR);
	db_path.exists()
}

/// Helper to derive XOR keys for storing private transaction keys in the DB
/// (blind_xor_key, nonce_xor_key)
fn private_ctx_xor_keys<K>(
	keychain: &K,
	slate_id: &[u8],
) -> Result<([u8; SECRET_KEY_SIZE], [u8; SECRET_KEY_SIZE]), Error>
where
	K: Keychain,
{
	let root_key = keychain.derive_key(0, &K::root_key_id(), SwitchCommitmentType::Regular)?;

	// derive XOR values for storing secret values in DB
	// h(root_key|slate_id|"blind")
	let mut hasher = Blake2b::new(SECRET_KEY_SIZE);
	hasher.update(&root_key.0[..]);
	hasher.update(&slate_id[..]);
	hasher.update(&b"blind"[..]);
	let blind_xor_key = hasher.finalize();
	let mut ret_blind = [0; SECRET_KEY_SIZE];
	ret_blind.copy_from_slice(&blind_xor_key.as_bytes()[0..SECRET_KEY_SIZE]);

	// h(root_key|slate_id|"nonce")
	let mut hasher = Blake2b::new(SECRET_KEY_SIZE);
	hasher.update(&root_key.0[..]);
	hasher.update(&slate_id[..]);
	hasher.update(&b"nonce"[..]);
	let nonce_xor_key = hasher.finalize();
	let mut ret_nonce = [0; SECRET_KEY_SIZE];
	ret_nonce.copy_from_slice(&nonce_xor_key.as_bytes()[0..SECRET_KEY_SIZE]);

	Ok((ret_blind, ret_nonce))
}

pub struct LMDBBackend<'ck, C, K>
where
	C: NodeClient + 'ck,
	K: Keychain + 'ck,
{
	db: store::Store,
	data_file_dir: String,
	/// Keychain
	pub keychain: Option<K>,
	/// Check value for XORed keychain seed
	pub master_checksum: Box<Option<Blake2bResult>>,
	/// Parent path to use by default for output operations
	parent_key_id: Identifier,
	/// wallet to node client
	w2n_client: C,
	/// ethereum wallet instance
	ethereum_wallet: Option<EthereumWallet>,
	///phantom
	_phantom: &'ck PhantomData<C>,
}

impl<'ck, C, K> LMDBBackend<'ck, C, K>
where
	C: NodeClient + 'ck,
	K: Keychain + 'ck,
{
	pub fn new(data_file_dir: &str, n_client: C) -> Result<Self, Error> {
		let db_path = path::Path::new(data_file_dir).join(DB_DIR);
		fs::create_dir_all(&db_path).expect("Couldn't create wallet backend directory!");

		let stored_tx_path = path::Path::new(data_file_dir).join(TX_SAVE_DIR);
		fs::create_dir_all(&stored_tx_path)
			.expect("Couldn't create wallet backend tx storage directory!");

		let store = store::Store::new(db_path.to_str().unwrap(), None, Some(DB_DIR), None)?;

		// Make sure default wallet derivation path always exists
		// as well as path (so it can be retrieved by batches to know where to store
		// completed transactions, for reference
		let default_account = AcctPathMapping {
			label: "default".to_owned(),
			path: LMDBBackend::<C, K>::default_path(),
		};
		let acct_key = to_key(
			ACCOUNT_PATH_MAPPING_PREFIX,
			&mut default_account.label.as_bytes().to_vec(),
		);

		{
			let batch = store.batch()?;
			batch.put_ser(&acct_key, &default_account)?;
			batch.commit()?;
		}

		TxProof::init_proof_backend(data_file_dir)?;

		let res = LMDBBackend {
			db: store,
			data_file_dir: data_file_dir.to_owned(),
			keychain: None,
			master_checksum: Box::new(None),
			parent_key_id: LMDBBackend::<C, K>::default_path(),
			w2n_client: n_client,
			ethereum_wallet: None,
			_phantom: &PhantomData,
		};
		Ok(res)
	}

	fn default_path() -> Identifier {
		// return the default parent wallet path, corresponding to the default account
		// in the BIP32 spec. Parent is account 0 at level 2, child output identifiers
		// are all at level 3
		ExtKeychain::derive_key_id(2, 0, 0, 0, 0)
	}

	/// Just test to see if database files exist in the current directory. If
	/// so, use a DB backend for all operations
	pub fn exists(data_file_dir: &str) -> bool {
		let db_path = path::Path::new(data_file_dir).join(DB_DIR);
		db_path.exists()
	}
}

impl<'ck, C, K> WalletBackend<'ck, C, K> for LMDBBackend<'ck, C, K>
where
	C: NodeClient + 'ck,
	K: Keychain + 'ck,
{
	/// data file directory. why713 needs it
	fn get_data_file_dir(&self) -> &str {
		&self.data_file_dir
	}

	/// Set the keychain, which should already have been opened
	fn set_keychain(
		&mut self,
		mut k: Box<K>,
		mask: bool,
		use_test_rng: bool,
	) -> Result<Option<SecretKey>, Error> {
		// store hash of master key, so it can be verified later after unmasking
		let root_key = k.derive_key(0, &K::root_key_id(), SwitchCommitmentType::Regular)?;
		let mut hasher = Blake2b::new(SECRET_KEY_SIZE);
		hasher.update(&root_key.0[..]);
		self.master_checksum = Box::new(Some(hasher.finalize()));

		let mask_value = {
			match mask {
				true => {
					// Random value that must be XORed against the stored wallet seed
					// before it is used
					let mask_value = match use_test_rng {
						true => {
							let mut test_rng = StepRng::new(1_234_567_890_u64, 1);
							secp::key::SecretKey::new(&mut test_rng)
						}
						false => secp::key::SecretKey::new(&mut thread_rng()),
					};
					k.mask_master_key(&mask_value)?;
					Some(mask_value)
				}
				false => None,
			}
		};

		self.keychain = Some(*k);
		Ok(mask_value)
	}

	/// Close wallet
	fn close(&mut self) -> Result<(), Error> {
		self.keychain = None;
		Ok(())
	}

	/// Return the keychain being used, cloned with XORed token value
	/// for temporary use
	fn keychain(&self, mask: Option<&SecretKey>) -> Result<K, Error> {
		match self.keychain.as_ref() {
			Some(k) => {
				let mut k_masked = k.clone();
				if let Some(m) = mask {
					k_masked.mask_master_key(m)?;
				}
				// Check if master seed is what is expected (especially if it's been xored)
				let root_key =
					k_masked.derive_key(0, &K::root_key_id(), SwitchCommitmentType::Regular)?;
				let mut hasher = Blake2b::new(SECRET_KEY_SIZE);
				hasher.update(&root_key.0[..]);
				if *self.master_checksum != Some(hasher.finalize()) {
					error!("Supplied keychain mask is invalid");
					return Err(ErrorKind::InvalidKeychainMask.into());
				}
				Ok(k_masked)
			}
			None => Err(ErrorKind::KeychainDoesntExist.into()),
		}
	}

	/// Return the node client being used
	fn w2n_client(&mut self) -> &mut C {
		&mut self.w2n_client
	}

	/// return the version of the commit for caching
	fn calc_commit_for_cache(
		&mut self,
		keychain_mask: Option<&SecretKey>,
		amount: u64,
		id: &Identifier,
	) -> Result<Option<String>, Error> {
		//TODO: Check if this is really necessary, it's the only thing
		//preventing removing the need for config in the wallet backend
		/*if self.config.no_commit_cache == Some(true) {
			Ok(None)
		} else {*/
		Ok(Some(util::to_hex(
			&self
				.keychain(keychain_mask)?
				.commit(amount, &id, SwitchCommitmentType::Regular)?
				.0, // TODO: proper support for different switch commitment schemes
		)))
		/*}*/
	}

	/// Set parent path by account name
	fn set_parent_key_id_by_name(&mut self, label: &str) -> Result<(), Error> {
		let label = label.to_owned();
		let res = self.acct_path_iter().find(|l| l.label == label);
		if let Some(a) = res {
			self.set_parent_key_id(a.path);
			Ok(())
		} else {
			Err(ErrorKind::UnknownAccountLabel(label).into())
		}
	}

	/// set parent path
	fn set_parent_key_id(&mut self, id: Identifier) {
		self.parent_key_id = id;
	}

	fn parent_key_id(&mut self) -> Identifier {
		self.parent_key_id.clone()
	}

	fn get(&self, id: &Identifier, mmr_index: &Option<u64>) -> Result<OutputData, Error> {
		let key = match mmr_index {
			Some(i) => to_key_u64(OUTPUT_PREFIX, &mut id.to_bytes().to_vec(), *i),
			None => to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec()),
		};
		option_to_not_found(self.db.get_ser(&key), || format!("Key Id: {}", id))
			.map_err(|e| e.into())
	}

	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = OutputData> + 'a> {
		Box::new(self.db.iter(&[OUTPUT_PREFIX]).unwrap().map(|o| o.1))
	}

	fn tx_log_iter<'a>(&'a self) -> Box<dyn Iterator<Item = TxLogEntry> + 'a> {
		Box::new(self.db.iter(&[TX_LOG_ENTRY_PREFIX]).unwrap().map(|o| o.1))
	}

	fn get_private_context(
		&mut self,
		keychain_mask: Option<&SecretKey>,
		slate_id: &[u8],
		participant_id: usize,
	) -> Result<Context, Error> {
		let ctx_key = to_key_u64(
			PRIVATE_TX_CONTEXT_PREFIX,
			&mut slate_id.to_vec(),
			participant_id as u64,
		);
		let (blind_xor_key, nonce_xor_key) =
			private_ctx_xor_keys(&self.keychain(keychain_mask)?, slate_id)?;

		let mut ctx: Context = option_to_not_found(self.db.get_ser(&ctx_key), || {
			format!("Slate id: {:x?}", slate_id.to_vec())
		})?;

		for i in 0..SECRET_KEY_SIZE {
			ctx.sec_key.0[i] ^= blind_xor_key[i];
			ctx.sec_nonce.0[i] ^= nonce_xor_key[i];
		}

		Ok(ctx)
	}

	fn acct_path_iter<'a>(&'a self) -> Box<dyn Iterator<Item = AcctPathMapping> + 'a> {
		Box::new(
			self.db
				.iter(&[ACCOUNT_PATH_MAPPING_PREFIX])
				.unwrap()
				.map(|o| o.1),
		)
	}

	fn get_acct_path(&self, label: String) -> Result<Option<AcctPathMapping>, Error> {
		let acct_key = to_key(ACCOUNT_PATH_MAPPING_PREFIX, &mut label.as_bytes().to_vec());
		self.db.get_ser(&acct_key).map_err(|e| e.into())
	}

	fn store_tx(&self, uuid: &str, tx: &Transaction) -> Result<(), Error> {
		let filename = format!("{}.whytx", uuid);
		let path = path::Path::new(&self.data_file_dir)
			.join(TX_SAVE_DIR)
			.join(filename);
		let path_buf = Path::new(&path).to_path_buf();
		let mut stored_tx = File::create(path_buf)?;
		let tx_hex = util::to_hex(&ser::ser_vec(tx, ser::ProtocolVersion(1))?);
		stored_tx.write_all(&tx_hex.as_bytes())?;
		stored_tx.sync_all()?;
		Ok(())
	}

	fn get_stored_tx(&self, entry: &TxLogEntry) -> Result<Option<Transaction>, Error> {
		let filename = match entry.stored_tx.clone() {
			Some(f) => f,
			None => return Ok(None),
		};
		let path = path::Path::new(&self.data_file_dir)
			.join(TX_SAVE_DIR)
			.join(filename);

		match path.to_str() {
			Some(s) => Ok(Some(self.load_stored_tx(s)?)),
			None => Err(ErrorKind::GenericError(
				"Unable to build transaction path".to_string(),
			))?,
		}
	}

	// why need to suport extentions whytx and grintx because 2.4 version has grintx, 3.0 whytx
	fn get_stored_tx_by_uuid(&self, uuid: &str) -> Result<Transaction, Error> {
		let get_stored_tx_by_uuid_ext =
			|uuid: &str, extention: &str| -> Result<Transaction, Error> {
				let filename = format!("{}.{}", uuid, extention);

				let path = path::Path::new(&self.data_file_dir)
					.join(TX_SAVE_DIR)
					.join(filename);

				let trans = self.load_stored_tx(path.to_str().ok_or(
					ErrorKind::GenericError("Unable to build transaction path".to_string()),
				)?)?;
				Ok(trans)
			};

		get_stored_tx_by_uuid_ext(uuid, "whytx")
			.or_else(|_| get_stored_tx_by_uuid_ext(uuid, "grintx"))
	}

	fn load_stored_tx(&self, path: &str) -> Result<Transaction, Error> {
		let tx_file = Path::new(&path).to_path_buf();
		let mut tx_f = File::open(tx_file)?;
		let mut content = String::new();
		tx_f.read_to_string(&mut content)?;
		let tx_bin = util::from_hex(&content).map_err(|e| {
			ErrorKind::StoredTransactionError(format!("Unable to decode the data, {}", e))
		})?;
		Ok(
			ser::deserialize(&mut &tx_bin[..], ser::ProtocolVersion(1)).map_err(|e| {
				ErrorKind::StoredTransactionError(format!("Unable to deserialize the data, {}", e))
			})?,
		)
	}

	fn batch<'a>(
		&'a mut self,
		keychain_mask: Option<&SecretKey>,
	) -> Result<Box<dyn WalletOutputBatch<K> + 'a>, Error> {
		Ok(Box::new(Batch {
			_store: self,
			db: RefCell::new(Some(self.db.batch()?)),
			keychain: Some(self.keychain(keychain_mask)?),
		}))
	}

	fn batch_no_mask<'a>(&'a mut self) -> Result<Box<dyn WalletOutputBatch<K> + 'a>, Error> {
		Ok(Box::new(Batch {
			_store: self,
			db: RefCell::new(Some(self.db.batch()?)),
			keychain: None,
		}))
	}

	fn current_child_index<'a>(&mut self, parent_key_id: &Identifier) -> Result<u32, Error> {
		let index = {
			let batch = self.db.batch()?;
			let deriv_key = to_key(DERIV_PREFIX, &mut parent_key_id.to_bytes().to_vec());
			match batch.get_ser(&deriv_key)? {
				Some(idx) => idx,
				None => 0,
			}
		};
		Ok(index)
	}

	fn next_child<'a>(
		&mut self,
		keychain_mask: Option<&SecretKey>,
		parent_key_id: Option<Identifier>,
		height: Option<u64>,
	) -> Result<Identifier, Error> {
		let parent_key_id = parent_key_id.unwrap_or(self.parent_key_id.clone());
		let mut deriv_idx = {
			let batch = self.db.batch()?;
			let deriv_key = to_key(DERIV_PREFIX, &mut self.parent_key_id.to_bytes().to_vec());
			match batch.get_ser(&deriv_key)? {
				Some(idx) => idx,
				None => 0,
			}
		};
		let mut return_path = self.parent_key_id.to_path();
		return_path.depth += 1;
		return_path.path[return_path.depth as usize - 1] = ChildNumber::from(deriv_idx);
		if let Some(hei) = height {
			//u32::max is 4294967295 based on the block generating speed(1 min/block)
			//it will take about 837 years for the height to go over the u32 range.
			return_path.path[3] = ChildNumber::from(hei as u32); //put the height in the last index.
		}
		deriv_idx += 1;
		let mut batch = self.batch(keychain_mask)?;
		batch.save_child_index(&parent_key_id, deriv_idx)?;
		batch.commit()?;
		Ok(Identifier::from_path(&return_path))
	}

	fn last_confirmed_height<'a>(&mut self) -> Result<u64, Error> {
		let batch = self.db.batch()?;
		let height_key = to_key(
			CONFIRMED_HEIGHT_PREFIX,
			&mut self.parent_key_id.to_bytes().to_vec(),
		);
		let last_confirmed_height = match batch.get_ser(&height_key)? {
			Some(h) => h,
			None => 0,
		};
		Ok(last_confirmed_height)
	}

	fn last_scanned_blocks<'a>(&mut self) -> Result<Vec<ScannedBlockInfo>, Error> {
		let batch = self.db.batch()?;
		let mut blocks: Vec<ScannedBlockInfo> = batch
			.iter(&[LAST_SCANNED_BLOCK])
			.unwrap()
			.map(|o| o.1)
			.collect();

		blocks.sort_by(|a, b| b.height.cmp(&a.height));

		debug!("last_scanned_blocks: {:?}", blocks);

		Ok(blocks)
	}

	/// set ethereum wallet instance
	fn set_ethereum_wallet(
		&mut self,
		ethereum_wallet: Option<EthereumWallet>,
	) -> Result<(), Error> {
		self.ethereum_wallet = ethereum_wallet;
		Ok(())
	}

	/// get ethereum wallet instance
	fn get_ethereum_wallet(&self) -> Result<EthereumWallet, Error> {
		if self.ethereum_wallet.is_some() {
			Ok(self.ethereum_wallet.clone().unwrap())
		} else {
			Err(
				ErrorKind::EthereumWalletError("Ethereum Wallet Not Generated!!!".to_string())
					.into(),
			)
		}
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a, C, K>
where
	C: NodeClient,
	K: Keychain,
{
	_store: &'a LMDBBackend<'a, C, K>,
	db: RefCell<Option<store::Batch<'a>>>,
	/// Keychain
	keychain: Option<K>,
}

#[allow(missing_docs)]
impl<'a, C, K> WalletOutputBatch<K> for Batch<'a, C, K>
where
	C: NodeClient,
	K: Keychain,
{
	fn keychain(&mut self) -> &mut K {
		self.keychain.as_mut().unwrap()
	}

	fn save(&mut self, out: OutputData) -> Result<(), Error> {
		// Save the output data to the db.
		{
			let key = match out.mmr_index {
				Some(i) => to_key_u64(OUTPUT_PREFIX, &mut out.key_id.to_bytes().to_vec(), i),
				None => to_key(OUTPUT_PREFIX, &mut out.key_id.to_bytes().to_vec()),
			};
			self.db.borrow().as_ref().unwrap().put_ser(&key, &out)?;
		}

		Ok(())
	}

	fn get(&self, id: &Identifier, mmr_index: &Option<u64>) -> Result<OutputData, Error> {
		let key = match mmr_index {
			Some(i) => to_key_u64(OUTPUT_PREFIX, &mut id.to_bytes().to_vec(), *i),
			None => to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec()),
		};
		option_to_not_found(self.db.borrow().as_ref().unwrap().get_ser(&key), || {
			format!("Key ID: {}", id)
		})
		.map_err(|e| e.into())
	}

	fn iter(&self) -> Box<dyn Iterator<Item = OutputData>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[OUTPUT_PREFIX])
				.unwrap()
				.map(|o| o.1),
		)
	}

	fn delete(&mut self, id: &Identifier, mmr_index: &Option<u64>) -> Result<(), Error> {
		// Delete the output data.
		{
			let key = match mmr_index {
				Some(i) => to_key_u64(OUTPUT_PREFIX, &mut id.to_bytes().to_vec(), *i),
				None => to_key(OUTPUT_PREFIX, &mut id.to_bytes().to_vec()),
			};
			let _ = self.db.borrow().as_ref().unwrap().delete(&key);
		}

		Ok(())
	}

	fn next_tx_log_id(&mut self, parent_key_id: &Identifier) -> Result<u32, Error> {
		let tx_id_key = to_key(TX_LOG_ID_PREFIX, &mut parent_key_id.to_bytes().to_vec());
		let last_tx_log_id = match self.db.borrow().as_ref().unwrap().get_ser(&tx_id_key)? {
			Some(t) => t,
			None => 0,
		};
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&tx_id_key, &(last_tx_log_id + 1))?;
		Ok(last_tx_log_id)
	}

	fn tx_log_iter(&self) -> Box<dyn Iterator<Item = TxLogEntry>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[TX_LOG_ENTRY_PREFIX])
				.unwrap()
				.map(|o| o.1),
		)
	}

	fn save_last_confirmed_height(
		&mut self,
		parent_key_id: &Identifier,
		height: u64,
	) -> Result<(), Error> {
		let height_key = to_key(
			CONFIRMED_HEIGHT_PREFIX,
			&mut parent_key_id.to_bytes().to_vec(),
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&height_key, &height)?;
		Ok(())
	}

	fn save_last_scanned_blocks(
		&mut self,
		first_scanned_block_height: u64,
		block_info: &Vec<ScannedBlockInfo>,
	) -> Result<(), Error> {
		debug_assert!(block_info.first().unwrap().height >= block_info.last().unwrap().height);

		let br = self.db.borrow();
		let db = br.as_ref().unwrap();

		// Cleaning up the head blocks...
		let mut heights: Vec<u64> = db
			.iter(&[LAST_SCANNED_BLOCK])
			.unwrap()
			.map(|o: (Vec<u8>, ScannedBlockInfo)| o.1.height)
			.collect();

		for h in &heights {
			if *h >= first_scanned_block_height {
				db.delete(&u64_to_key(LAST_SCANNED_BLOCK, *h))?;
			}
		}

		heights.retain(|h| *h < first_scanned_block_height);

		// Inserting the new data
		for bl_info in block_info {
			let scan_block_key = u64_to_key(LAST_SCANNED_BLOCK, bl_info.height);
			db.put_ser(&scan_block_key, bl_info)?;
		}

		heights.extend(block_info.iter().map(|b| b.height));
		heights.sort();

		let mut step = 4;
		let mut start = heights.pop().unwrap_or(1);

		while let Some(h) = heights.pop() {
			assert!(h < start);
			if start - h < step {
				db.delete(&u64_to_key(LAST_SCANNED_BLOCK, h))?;
			} else {
				start = h;
				step *= 2;
			}
		}

		Ok(())
	}

	/// Save the last used good node index
	fn save_last_working_node_index(&mut self, node_index: u8) -> Result<(), Error> {
		let node_index_key = u64_to_key(LAST_WORKING_NODE_INDEX, 0 as u64);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&node_index_key, &node_index)?;
		Ok(())
	}

	/// Save the last used good node index
	fn get_last_working_node_index(&mut self) -> Result<u8, Error> {
		let node_index_key = u64_to_key(LAST_WORKING_NODE_INDEX, 0 as u64);

		let index: Option<u8> = self
			.db
			.borrow()
			.as_ref()
			.unwrap()
			.get_ser(&node_index_key)?;
		let last_working_node_index = match index {
			Some(ind) => ind as u8, //the normal index started from 1. 0 is error
			None => 0,
		};
		Ok(last_working_node_index)
	}

	fn save_child_index(&mut self, parent_id: &Identifier, child_n: u32) -> Result<(), Error> {
		let deriv_key = to_key(DERIV_PREFIX, &mut parent_id.to_bytes().to_vec());
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&deriv_key, &child_n)?;
		Ok(())
	}

	fn save_tx_log_entry(
		&mut self,
		tx_in: TxLogEntry,
		parent_id: &Identifier,
	) -> Result<(), Error> {
		let tx_log_key = to_key_u64(
			TX_LOG_ENTRY_PREFIX,
			&mut parent_id.to_bytes().to_vec(),
			tx_in.id as u64,
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&tx_log_key, &tx_in)?;
		Ok(())
	}

	fn rename_acct_path(
		&mut self,
		accounts: Vec<AcctPathMapping>,
		old_name: &str,
		new_name: &str,
	) -> Result<(), Error> {
		for acc in accounts {
			if acc.label == old_name {
				let mut nacc = acc.clone();
				let old_key = to_key(
					ACCOUNT_PATH_MAPPING_PREFIX,
					&mut acc.label.as_bytes().to_vec(),
				);
				self.db.borrow().as_ref().unwrap().delete(&old_key)?;
				nacc.label = new_name.to_string();
				let acct_key = to_key(
					ACCOUNT_PATH_MAPPING_PREFIX,
					&mut nacc.label.as_bytes().to_vec(),
				);
				self.db
					.borrow()
					.as_ref()
					.unwrap()
					.put_ser(&acct_key, &nacc)?;

				break;
			}
		}
		println!("rename acct from '{}' to '{}'", old_name, new_name);
		Ok(())
	}

	fn save_acct_path(&mut self, mapping: AcctPathMapping) -> Result<(), Error> {
		let acct_key = to_key(
			ACCOUNT_PATH_MAPPING_PREFIX,
			&mut mapping.label.as_bytes().to_vec(),
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&acct_key, &mapping)?;
		Ok(())
	}

	fn acct_path_iter(&self) -> Box<dyn Iterator<Item = AcctPathMapping>> {
		Box::new(
			self.db
				.borrow()
				.as_ref()
				.unwrap()
				.iter(&[ACCOUNT_PATH_MAPPING_PREFIX])
				.unwrap()
				.map(|o| o.1),
		)
	}

	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), Error> {
		out.lock();
		self.save(out.clone())
	}

	fn save_private_context(
		&mut self,
		slate_id: &[u8],
		participant_id: usize,
		ctx: &Context,
	) -> Result<(), Error> {
		let ctx_key = to_key_u64(
			PRIVATE_TX_CONTEXT_PREFIX,
			&mut slate_id.to_vec(),
			participant_id as u64,
		);
		let (blind_xor_key, nonce_xor_key) = private_ctx_xor_keys(self.keychain(), slate_id)?;

		let mut s_ctx = ctx.clone();
		for i in 0..SECRET_KEY_SIZE {
			s_ctx.sec_key.0[i] ^= blind_xor_key[i];
			s_ctx.sec_nonce.0[i] ^= nonce_xor_key[i];
		}

		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&ctx_key, &s_ctx)?;
		Ok(())
	}

	fn delete_private_context(
		&mut self,
		slate_id: &[u8],
		participant_id: usize,
	) -> Result<(), Error> {
		let ctx_key = to_key_u64(
			PRIVATE_TX_CONTEXT_PREFIX,
			&mut slate_id.to_vec(),
			participant_id as u64,
		);
		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.delete(&ctx_key)
			.map_err(|e| e.into())
	}

	fn commit(&self) -> Result<(), Error> {
		let db = self.db.replace(None);
		db.unwrap().commit()?;
		Ok(())
	}

	fn save_integrity_context(
		&mut self,
		slate_id: &[u8],
		ctx: &IntegrityContext,
	) -> Result<(), Error> {
		let ctx_key = to_key(INTEGRITY_CONTEXT_PREFIX, &mut slate_id.to_vec());
		let (blind_xor_key, _nonce_xor_key) = private_ctx_xor_keys(self.keychain(), slate_id)?;

		let mut s_ctx = ctx.clone();
		for i in 0..SECRET_KEY_SIZE {
			s_ctx.sec_key.0[i] ^= blind_xor_key[i];
		}

		self.db
			.borrow()
			.as_ref()
			.unwrap()
			.put_ser(&ctx_key, &s_ctx)?;
		Ok(())
	}

	fn load_integrity_context(&mut self, slate_id: &[u8]) -> Result<IntegrityContext, Error> {
		let ctx_key = to_key(INTEGRITY_CONTEXT_PREFIX, &mut slate_id.to_vec());
		let (blind_xor_key, _nonce_xor_key) = private_ctx_xor_keys(self.keychain(), slate_id)?;

		let mut ctx: IntegrityContext =
			option_to_not_found(self.db.borrow().as_ref().unwrap().get_ser(&ctx_key), || {
				format!("Slate id: {:x?}", slate_id.to_vec())
			})?;

		for i in 0..SECRET_KEY_SIZE {
			ctx.sec_key.0[i] ^= blind_xor_key[i];
		}

		Ok(ctx)
	}
}
