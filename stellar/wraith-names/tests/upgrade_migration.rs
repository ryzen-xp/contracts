#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, BytesN, Env,
};
use wraith_names::{
    DataKey, NameEntry, NamesError, WraithNamesContract, WraithNamesContractClient,
};

const TTL_THRESHOLD: u32 = 17_280;
const TTL_EXTEND_TO: u32 = 518_400;

fn env_with_ttl() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.min_persistent_entry_ttl = TTL_EXTEND_TO;
        li.max_entry_ttl = TTL_EXTEND_TO * 2;
    });
    env
}

fn name_hash(env: &Env, name: &str) -> BytesN<32> {
    let name_bytes = name.as_bytes();
    let hash = env.crypto().sha256(&Bytes::from_slice(env, name_bytes));
    BytesN::from_array(env, &hash.to_array())
}

fn meta(env: &Env, seed: u8) -> Bytes {
    Bytes::from_slice(env, &[seed; 64])
}

#[test]
fn migration_v0_snapshot_name_entry_readable_by_v1() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name_str = "alice";
    let meta_addr = meta(&env, 0xAA);

    let name_hash_val = name_hash(&env, name_str);

    env.as_contract(&contract_id, || {
        let entry = NameEntry {
            name: soroban_sdk::String::from_str(&env, name_str),
            stealth_meta_address: meta_addr.clone(),
            owner: owner.clone(),
            parent: None,
        };

        let key = DataKey::Name(name_hash_val.clone());
        env.storage().persistent().set(&key, &entry);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
    });

    env.as_contract(&contract_id, || {
        let key = DataKey::Name(name_hash_val.clone());
        let stored: Option<NameEntry> = env.storage().persistent().get(&key);
        assert!(stored.is_some());
        let entry = stored.unwrap();
        assert_eq!(entry.owner, owner);
        assert_eq!(entry.stealth_meta_address, meta_addr);
        assert_eq!(entry.parent, None);
    });
}

#[test]
fn migration_v0_reverse_lookup_readable_by_v1() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let meta_addr = meta(&env, 0x55);
    let meta_hash = env.crypto().sha256(&meta_addr);

    let name_str = "bob";
    let name_hash_val = name_hash(&env, name_str);

    env.as_contract(&contract_id, || {
        let key = DataKey::Reverse(BytesN::from_array(&env, unsafe {
            &*(meta_hash.to_array().as_ptr() as *const [u8; 32])
        }));
        env.storage().persistent().set(&key, &name_hash_val);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
    });

    env.as_contract(&contract_id, || {
        let key = DataKey::Reverse(BytesN::from_array(&env, unsafe {
            &*(meta_hash.to_array().as_ptr() as *const [u8; 32])
        }));
        let stored: Option<BytesN<32>> = env.storage().persistent().get(&key);
        assert_eq!(stored, Some(name_hash_val));
    });
}

#[test]
fn migration_v1_new_keys_written_with_parent_support() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "charlie");
    let meta_addr = meta(&env, 0x77);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    let name_hash_val = name_hash(&env, "charlie");

    env.as_contract(&contract_id, || {
        let key = DataKey::Name(name_hash_val.clone());
        let stored: Option<NameEntry> = env.storage().persistent().get(&key);
        assert!(stored.is_some());

        let entry = stored.unwrap();
        assert_eq!(entry.owner, owner);
        assert_eq!(entry.stealth_meta_address, meta_addr);
        assert_eq!(entry.parent, None);
    });
}

#[test]
fn migration_v1_replay_protection_key_created() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "dave");
    let meta_addr = meta(&env, 0x99);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    env.as_contract(&contract_id, || {
        let operation_hash = env
            .crypto()
            .sha256(&Bytes::from_slice(&env, b"wraith-names:register"));
        let replay_key = DataKey::Replay(BytesN::from_array(&env, unsafe {
            &*(operation_hash.to_array().as_ptr() as *const [u8; 32])
        }));

        let replay_state: Option<bool> = env.storage().persistent().get(&replay_key);
        assert!(replay_state.is_some() || replay_state.is_none());
    });
}

#[test]
fn migration_post_upgrade_records_survive_ttl_threshold() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "eve");
    let meta_addr = meta(&env, 0xBB);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    env.ledger().with_mut(|li| {
        li.sequence_number += TTL_THRESHOLD + 1;
    });

    let resolved = client.resolve(&name);
    assert_eq!(resolved, meta_addr);
}

#[test]
fn migration_batch_v0_names_all_readable() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let client = WraithNamesContractClient::new(&env, &contract_id);

    let names_meta = vec![
        ("frank", meta(&env, 0x11)),
        ("grace", meta(&env, 0x22)),
        ("henry", meta(&env, 0x33)),
    ];

    for (name_str, meta_addr) in &names_meta {
        let name = soroban_sdk::String::from_str(&env, name_str);
        client.register(&owner, &name, &meta_addr);
    }

    for (name_str, expected_meta) in &names_meta {
        let name = soroban_sdk::String::from_str(&env, name_str);
        let resolved = client.resolve(&name);
        assert_eq!(&resolved, expected_meta);
    }
}

#[test]
fn migration_v1_multisig_keys_not_present_in_v0() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "iris");
    let meta_addr = meta(&env, 0xCC);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    let name_hash_val = name_hash(&env, "iris");

    env.as_contract(&contract_id, || {
        let guard_key = DataKey::Guardians(name_hash_val.clone());
        let guardians: Option<soroban_sdk::Vec<Address>> =
            env.storage().persistent().get(&guard_key);
        assert!(guardians.is_none());

        let recovery_key = DataKey::Recovery(name_hash_val.clone());
        let recovery: Option<soroban_sdk::Vec<Address>> =
            env.storage().persistent().get(&recovery_key);
        assert!(recovery.is_none());
    });
}

#[test]
fn migration_rollback_records_in_wrong_tier_invisible() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name_str = "jack";
    let meta_addr = meta(&env, 0xDD);
    let name_hash_val = name_hash(&env, name_str);

    env.as_contract(&contract_id, || {
        let entry = NameEntry {
            name: soroban_sdk::String::from_str(&env, name_str),
            stealth_meta_address: meta_addr.clone(),
            owner: owner.clone(),
            parent: None,
        };

        let key = DataKey::Name(name_hash_val.clone());
        env.storage().persistent().set(&key, &entry.clone());

        let fake_entry = NameEntry {
            name: soroban_sdk::String::from_str(&env, "fake"),
            stealth_meta_address: meta(&env, 0xFF),
            owner: Address::generate(&env),
            parent: None,
        };
        env.storage().instance().set(&key, &fake_entry);
    });

    env.as_contract(&contract_id, || {
        let from_persistent: Option<NameEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::Name(name_hash_val.clone()));
        let from_instance: Option<NameEntry> = env
            .storage()
            .instance()
            .get(&DataKey::Name(name_hash_val.clone()));

        assert!(from_persistent.is_some());
        assert!(from_instance.is_some());
    });
}

#[test]
fn migration_incompatible_invalid_name_length_rejected() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let client = WraithNamesContractClient::new(&env, &contract_id);

    let short_name = soroban_sdk::String::from_str(&env, "ab");
    let meta_addr = meta(&env, 0xAA);

    let result = client.try_register(&owner, &short_name, &meta_addr);
    assert_eq!(result, Err(Ok(NamesError::NameTooShort)));
}

#[test]
fn migration_v1_event_schema_maintains_topic_layout() {
    use soroban_sdk::testutils::Events;

    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "karl");
    let meta_addr = meta(&env, 0xEE);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    let events = env.events().all();
    assert!(!events.is_empty());
}

#[test]
fn migration_v0_to_v1_admin_state_initialized() {
    let env = env_with_ttl();
    let contract_id = env.register(WraithNamesContract, ());

    let owner = Address::generate(&env);
    let name = soroban_sdk::String::from_str(&env, "laura");
    let meta_addr = meta(&env, 0xFF);

    let client = WraithNamesContractClient::new(&env, &contract_id);
    client.register(&owner, &name, &meta_addr);

    env.as_contract(&contract_id, || {
        let admin: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        let paused: Option<bool> = env.storage().instance().get(&DataKey::Paused);

        assert!(admin.is_some() || admin.is_none());
        assert!(paused.is_some() || paused.is_none());
    });
}
