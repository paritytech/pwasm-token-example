#![cfg_attr(not(feature="std"), no_std)]
#![allow(non_snake_case)]

extern crate tiny_keccak;
extern crate pwasm_std;
extern crate pwasm_ethereum;
extern crate pwasm_abi;
extern crate pwasm_abi_derive;

use tiny_keccak::Keccak;
use pwasm_ethereum as eth;
use pwasm_abi::types::*;
use pwasm_abi_derive::eth_abi;

// `TokenContract` is an interface definition of a contract.
// The following example covers the minimal subset of ERC20 token standard.
// eth_abi macro parses an interface (trait) definition of a contact and generates
// two structs: `Endpoint` and `Client`.
//
// `Endpoint` is an entry point for contract calls.
// eth_abi macro generates a table of Method IDs corresponding with every method signature defined in the trait
// and defines it statically in the generated code.
// See https://github.com/paritytech/pwasm-token-example/blob/master/src/token.rs how generated `Endpoint` is used in `call` and `deploy` (constructor).
// `Endpoint` instantiates with a struct TokenContractInstance which implements the trait definition.
//
// `Client` is a struct which is useful for call generation to a deployed contract. For example:
// ```
//     let mut client = Client::new(contactAddress);
//     let balance = client
//        .value(someValue) // you can attach some value for a call optionally
//        .balanceOf(someAddress);
// ```
// Will generate a Solidity-compatible call for the contract, deployed on `contactAddress`.
// Then it invokes pwasm_std::eth::call on `contactAddress` and returns the result.
#[eth_abi(Endpoint, Client)]
pub trait TokenContract {
	fn constructor(&mut self, _total_supply: U256);

	/// What is the balance of a particular account?
	#[constant]
	fn balanceOf(&mut self, _owner: Address) -> U256;

	/// Total amount of tokens
	#[constant]
	fn totalSupply(&mut self) -> U256;

	/// Transfer the balance from owner's account to another account
	fn transfer(&mut self, _to: Address, _amount: U256) -> bool;

	/// Send _value amount of tokens from address _from to address _to
	/// The transferFrom method is used for a withdraw workflow, allowing contracts to send
	/// tokens on your behalf, for example to "deposit" to a contract address and/or to charge
	/// fees in sub-currencies; the command should fail unless the _from account has
	/// deliberately authorized the sender of the message via some mechanism; we propose
	/// these standardized APIs for approval:
	fn transferFrom(&mut self, _from: Address, _to: Address, _amount: U256) -> bool;

	/// Allow _spender to withdraw from your account, multiple times, up to the _value amount.
	/// If this function is called again it overwrites the current allowance with _value.
	fn approve(&mut self, _spender: Address, _value: U256) -> bool;

	/// Check the amount of tokens spender have right to spend on behalf of owner
	fn allowance(&mut self, _owner: Address, _spender: Address) -> U256;

	#[event]
	fn Transfer(&mut self, indexed_from: Address, indexed_to: Address, _value: U256);
	#[event]
	fn Approval(&mut self, indexed_owner: Address, indexed_spender: Address, _value: U256);
}

lazy_static::lazy_static! {
	static ref TOTAL_SUPPLY_KEY: H256 =
		H256::from([2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
	static ref OWNER_KEY: H256 =
		H256::from([3,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
}

// Reads balance by address
fn read_balance_of(owner: &Address) -> U256 {
	U256::from_big_endian(&pwasm_ethereum::read(&balance_key(owner)))
}

// Generates a balance key for some address.
// Used to map balances with their owners.
fn balance_key(address: &Address) -> H256 {
	let mut key = H256::from(*address);
	key.as_bytes_mut()[0] = 1; // just a naive "namespace";
	key
}

// Reads allowance value using key
// Key generated by allowance_key function
fn read_allowance(key: &H256) -> U256 {
	U256::from_big_endian(&eth::read(key))
}

// Writes allowance value
// Key generated by allowance_key function
fn write_allowance(key: &H256, value: U256) {
	eth::write(key, &value.into())
}

// Generates the "allowance" storage key to map owner and spender
fn allowance_key(owner: &Address, spender: &Address) -> H256 {
	let mut keccak = Keccak::new_keccak256();
	let mut res = H256::zero();
	keccak.update("allowance_key".as_ref());
	keccak.update(owner.as_ref());
	keccak.update(spender.as_ref());
	keccak.finalize(res.as_bytes_mut());
	res
}

pub struct TokenContractInstance;

impl TokenContract for TokenContractInstance {
	fn constructor(&mut self, total_supply: U256) {
		let sender = eth::sender();
		// Set up the total supply for the token
		eth::write(&TOTAL_SUPPLY_KEY, &total_supply.into());
		// Give all tokens to the contract owner
		eth::write(&balance_key(&sender), &total_supply.into());
		// Set the contract owner
		eth::write(&OWNER_KEY, &H256::from(sender).into());
	}

	fn balanceOf(&mut self, owner: Address) -> U256 {
		read_balance_of(&owner)
	}

	fn totalSupply(&mut self) -> U256 {
		U256::from_big_endian(&eth::read(&TOTAL_SUPPLY_KEY))
	}

	fn transfer(&mut self, to: Address, amount: U256) -> bool {
		let sender = eth::sender();
		let senderBalance = read_balance_of(&sender);
		let recipientBalance = read_balance_of(&to);
		if amount == 0.into() || senderBalance < amount || to == sender {
			false
		} else {
			let new_sender_balance = senderBalance - amount;
			let new_recipient_balance = recipientBalance + amount;
			// TODO: impl From<U256> for H256 makes convertion to big endian. Could be optimized
			eth::write(&balance_key(&sender), &new_sender_balance.into());
			eth::write(&balance_key(&to), &new_recipient_balance.into());
			self.Transfer(sender, to, amount);
			true
		}
	}

	fn approve(&mut self, spender: Address, value: U256) -> bool {
		write_allowance(&allowance_key(&eth::sender(), &spender), value);
		self.Approval(eth::sender(), spender, value);
		true
	}

	fn allowance(&mut self, owner: Address, spender: Address) -> U256 {
		read_allowance(&allowance_key(&owner, &spender))
	}

	fn transferFrom(&mut self, from: Address, to: Address, amount: U256) -> bool {
		let fromBalance = read_balance_of(&from);
		let recipientBalance = read_balance_of(&to);
		let a_key = allowance_key(&from, &eth::sender());
		let allowed = read_allowance(&a_key);
		if  allowed < amount || amount == 0.into() || fromBalance < amount  || to == from {
			false
		} else {
			let new_allowed = allowed - amount;
			let new_from_balance = fromBalance - amount;
			let new_recipient_balance = recipientBalance + amount;
			eth::write(&a_key, &new_allowed.into());
			eth::write(&balance_key(&from), &new_from_balance.into());
			eth::write(&balance_key(&to), &new_recipient_balance.into());
			self.Transfer(from, to, amount);
			true
		}
	}
}

#[cfg(test)]
extern crate pwasm_test;

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
	extern crate std;
	use tests::std::str::FromStr;
	use super::*;
	use pwasm_test::{ext_reset, ext_update, ext_get, External};

	#[test]
	fn balanceOf_should_return_balance() {
		ext_reset(|e| e.storage([
					1,0,0,0,0,0,0,0,0,0,0,0,
					31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31
				].into(),
				U256::from(100000).into()
			)
		);
		let address = Address::from([31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31,31]);
		let mut contract = TokenContractInstance{};
		assert_eq!(contract.balanceOf(address), 100000.into());
	}

	#[test]
	fn totalSupply_should_return_total_supply_contract_was_initialized_with() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		let total_supply = 42.into();
		contract.constructor(total_supply);
		assert_eq!(contract.totalSupply(), total_supply);
	}

	#[test]
	fn should_succeed_in_creating_max_possible_amount_of_tokens() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		// set total supply to maximum value of an unsigned 256 bit integer
		let total_supply =
			U256::from_dec_str("115792089237316195423570985008687907853269984665640564039457584007913129639935").unwrap();
		assert_eq!(total_supply, U256::max_value());
		contract.constructor(total_supply);
		assert_eq!(contract.totalSupply(), total_supply);
	}

	#[test]
	fn should_initially_give_the_total_supply_to_the_creator() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		let total_supply = 10000.into();
		contract.constructor(total_supply);
		assert_eq!(contract.balanceOf(ext_get().sender()), total_supply);
	}

	#[test]
	fn should_succeed_transfering_1000_from_owner_to_another_address() {
		let mut contract = TokenContractInstance{};

		let owner_address = Address::from_str("ea674fdde714fd979de3edf0f56aa9716b898ec8").unwrap();
		let sam_address = Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap();

		ext_reset(|e| e.sender(owner_address.clone()));

		let total_supply = 10000.into();
		contract.constructor(total_supply);

		assert_eq!(contract.balanceOf(owner_address), total_supply);

		assert_eq!(contract.transfer(sam_address, 1000.into()), true);
		assert_eq!(ext_get().logs().len(), 1);
		assert_eq!(ext_get().logs()[0].topics.as_ref(), &[
			// hash of the event name
			H256::from_str("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef").unwrap(),
			// sender address
			H256::from_str("000000000000000000000000ea674fdde714fd979de3edf0f56aa9716b898ec8").unwrap(),
			// recipient address
			H256::from_str("000000000000000000000000db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap()
		]);
		assert_eq!(ext_get().logs()[0].data.as_ref(), &[
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232]);
		assert_eq!(contract.balanceOf(owner_address), 9000.into());
		assert_eq!(contract.balanceOf(sam_address), 1000.into());
	}

	#[test]
	fn should_return_false_transfer_not_sufficient_funds() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		contract.constructor(10000.into());
		assert_eq!(contract.transfer(Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap(), 50000.into()), false);
		assert_eq!(contract.balanceOf(::pwasm_ethereum::sender()), 10000.into());
		assert_eq!(contract.balanceOf(Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap()), 0.into());
		assert_eq!(ext_get().logs().len(), 0, "Should be no events created");
	}

	#[test]
	fn approve_should_approve() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		let spender = Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap();
		contract.constructor(40000.into());
		contract.approve(spender, 40000.into());
		assert_eq!(ext_get().logs().len(), 1, "Should be 1 event logged");
		assert_eq!(ext_get().logs()[0].topics.as_ref(), &[
			// hash of the event name
			H256::from_str("8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925").unwrap(),
			// sender (owner) address
			H256::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
			// spender address
			H256::from_str("000000000000000000000000db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap()
		]);
		assert_eq!(contract.allowance(::pwasm_ethereum::sender(), spender.clone()), 40000.into());
	}

	#[test]
	fn spender_should_be_able_to_spend_if_allowed() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		let owner = Address::zero();
		let spender =
			Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap();
		let samAddress =
			Address::from_str("ea674fdde714fd979de3edf0f56aa9716b898ec8").unwrap();
		contract.constructor(40000.into());
		contract.approve(spender, 10000.into());

		// Build different external with sender = spender
		ext_update(|e| e.sender(spender));

		assert_eq!(contract.transferFrom(owner.clone(), samAddress.clone(), 5000.into()), true);
		assert_eq!(contract.balanceOf(samAddress.clone()), 5000.into());
		assert_eq!(contract.balanceOf(owner.clone()), 35000.into());

		assert_eq!(contract.transferFrom(owner.clone(), samAddress.clone(), 5000.into()), true);
		assert_eq!(contract.balanceOf(samAddress.clone()), 10000.into());
		assert_eq!(contract.balanceOf(owner.clone()), 30000.into());

		// The limit has reached. No more coins should be available to spend for the spender
		assert_eq!(contract.transferFrom(owner.clone(), samAddress.clone(), 1.into()), false);
		assert_eq!(contract.balanceOf(samAddress.clone()), 10000.into());
		assert_eq!(contract.balanceOf(owner.clone()), 30000.into());
		assert_eq!(ext_get().logs().len(), 3, "Two events should be created");
	}

	#[test]
	fn spender_should_not_be_able_to_spend_if_owner_has_no_coins() {
		ext_reset(|e| e);
		let mut contract = TokenContractInstance{};
		let owner = Address::zero();
		let spender =
			Address::from_str("db6fd484cfa46eeeb73c71edee823e4812f9e2e1").unwrap();
		let samAddress =
			Address::from_str("ea674fdde714fd979de3edf0f56aa9716b898ec8").unwrap();
		contract.constructor(70000.into());
		contract.transfer(samAddress, 30000.into());
		contract.approve(spender, 40000.into());

		// Build different external with sender = spender
		ext_update(|e| e.sender(spender));

		// Despite of the allowance, can't transfer because the owner is out of tokens
		assert_eq!(contract.transferFrom(owner.clone(), samAddress.clone(), 40001.into()), false);
		assert_eq!(contract.balanceOf(samAddress.clone()), 30000.into());
		assert_eq!(contract.balanceOf(owner.clone()), 40000.into());
		assert_eq!(ext_get().logs().len(), 2, "Should be no events created");
	}

	#[test]
	fn should_not_transfer_to_self() {
		let mut contract = TokenContractInstance{};
		let owner_address =
			Address::from_str("ea674fdde714fd979de3edf0f56aa9716b898ec8").unwrap();
		ext_reset(|e| e.sender(owner_address.clone()));
		let total_supply = 10000.into();
		contract.constructor(total_supply);
		assert_eq!(contract.balanceOf(owner_address), total_supply);
		assert_eq!(contract.transfer(owner_address, 1000.into()), false);
		assert_eq!(contract.transferFrom(owner_address, owner_address, 1000.into()), false);
		assert_eq!(contract.balanceOf(owner_address), 10000.into());
		assert_eq!(ext_get().logs().len(), 0);
	}
}
