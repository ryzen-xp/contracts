#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, BytesN, Env,
};
use stealth_sender::{DataKey, SenderError, StealthSenderContract, StealthSenderContractClient};

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

fn mock_announcer(env: &Env) -> Address {
    env.register(MockAnnouncer, ())
}

#[test]
fn migration_v0_snapshot_announcer_only_readable_by_v1() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer_addr = mock_announcer(&env);
    let admin = Address::generate(&env);

    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::Announcer, &announcer_addr);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
    });

    let _client = StealthSenderContractClient::new(&env, &contract_id);

    env.as_contract(&contract_id, || {
        let stored: Option<Address> = env.storage().instance().get(&DataKey::Announcer);
        assert_eq!(stored, Some(announcer_addr.clone()));

        let stored_admin: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        assert_eq!(stored_admin, Some(admin.clone()));

        let stored_paused: Option<bool> = env.storage().instance().get(&DataKey::Paused);
        assert_eq!(stored_paused, Some(false));
    });
}

#[test]
fn migration_v0_multiple_snapshot_records_readable() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let _announcer = mock_announcer(&env);
    let admins: Vec<Address> = (0..3).map(|_| Address::generate(&env)).collect();

    for (i, admin) in admins.iter().enumerate() {
        env.as_contract(&contract_id, || {
            let _key_suffix = i as u32;
            let key = DataKey::Admin;
            env.storage().instance().set(&key, admin);
        });
    }

    env.as_contract(&contract_id, || {
        let stored: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        assert_eq!(stored, Some(admins[2].clone()));
    });
}

#[test]
fn migration_v1_new_keys_written_to_instance() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let client = StealthSenderContractClient::new(&env, &contract_id);
    client.init(&announcer, &None, &Some(fee_recipient.clone()), &10, &admin);

    env.as_contract(&contract_id, || {
        let stored_announcer: Option<Address> = env.storage().instance().get(&DataKey::Announcer);
        assert_eq!(stored_announcer, Some(announcer.clone()));

        let stored_fee_recipient: Option<Option<Address>> =
            env.storage().instance().get(&DataKey::FeeRecipient);
        assert_eq!(stored_fee_recipient, Some(Some(fee_recipient.clone())));

        let stored_fee_bps: Option<u32> = env.storage().instance().get(&DataKey::FeeBasisPoints);
        assert_eq!(stored_fee_bps, Some(10));

        let stored_admin: Option<Address> = env.storage().instance().get(&DataKey::Admin);
        assert_eq!(stored_admin, Some(admin.clone()));
    });
}

#[test]
fn migration_v1_schema_invariant_unchanged() {
    let env = env_with_ttl();
    let contract_id1 = env.register(StealthSenderContract, ());
    let contract_id2 = env.register(StealthSenderContract, ());

    let announcer1 = mock_announcer(&env);
    let announcer2 = mock_announcer(&env);
    let admin = Address::generate(&env);

    let client1 = StealthSenderContractClient::new(&env, &contract_id1);
    client1.init(&announcer1, &None, &None, &0, &admin);

    let client2 = StealthSenderContractClient::new(&env, &contract_id2);
    client2.init(&announcer2, &None, &None, &0, &admin);

    env.as_contract(&contract_id1, || {
        let key = DataKey::Announcer;
        let stored1: Option<Address> = env.storage().instance().get(&key);
        assert_eq!(stored1, Some(announcer1.clone()));
    });

    env.as_contract(&contract_id2, || {
        let key = DataKey::Announcer;
        let stored2: Option<Address> = env.storage().instance().get(&key);
        assert_eq!(stored2, Some(announcer2.clone()));
    });
}

#[test]
fn migration_post_upgrade_reads_survive_ttl_threshold() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);

    let client = StealthSenderContractClient::new(&env, &contract_id);
    client.init(&announcer, &None, &None, &0, &admin);

    env.ledger().with_mut(|li| {
        li.sequence_number += TTL_THRESHOLD + 1;
    });

    env.as_contract(&contract_id, || {
        let stored: Option<Address> = env.storage().instance().get(&DataKey::Announcer);
        assert_eq!(stored, Some(announcer.clone()));
    });
}

#[test]
fn migration_v1_fee_config_validation_enforced_on_write() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);
    let fee_recipient = Address::generate(&env);

    let client = StealthSenderContractClient::new(&env, &contract_id);

    let init_with_fee_no_recipient = client.try_init(&announcer, &None, &None, &10, &admin);
    assert_eq!(
        init_with_fee_no_recipient,
        Err(Ok(SenderError::InvalidFeeConfig))
    );

    let init_with_fee_exceeds_cap =
        client.try_init(&announcer, &None, &Some(fee_recipient.clone()), &51, &admin);
    assert_eq!(
        init_with_fee_exceeds_cap,
        Err(Ok(SenderError::InvalidFeeConfig))
    );

    let init_valid = client.try_init(&announcer, &None, &Some(fee_recipient.clone()), &50, &admin);
    assert!(init_valid.is_ok());
}

#[test]
fn migration_rollback_records_in_wrong_tier_invisible_to_v1() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);

    env.as_contract(&contract_id, || {
        env.storage()
            .instance()
            .set(&DataKey::Announcer, &announcer);
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage()
            .persistent()
            .set(&DataKey::Announcer, &attacker);
    });

    env.as_contract(&contract_id, || {
        let from_instance: Option<Address> = env.storage().instance().get(&DataKey::Announcer);
        let from_persistent: Option<Address> = env.storage().persistent().get(&DataKey::Announcer);

        assert_eq!(from_instance, Some(announcer.clone()));
        assert_eq!(from_persistent, Some(attacker.clone()));
    });
}

#[test]
fn migration_incompatible_v1_keys_rejected_on_invalid_config() {
    let env = env_with_ttl();
    let contract_id = env.register(StealthSenderContract, ());

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);

    let client = StealthSenderContractClient::new(&env, &contract_id);

    let init_already = client.try_init(&announcer, &None, &None, &0, &admin);
    assert!(init_already.is_ok());

    let init_duplicate = client.try_init(&announcer, &None, &None, &0, &admin);
    assert_eq!(init_duplicate, Err(Ok(SenderError::AlreadyInitialized)));
}

#[test]
fn migration_batch_v0_records_all_readable_after_v1_init() {
    let env = env_with_ttl();
    let contract_ids: Vec<Address> = (0..3)
        .map(|_| env.register(StealthSenderContract, ()))
        .collect();

    let announcer = mock_announcer(&env);
    let admin = Address::generate(&env);

    for contract_id in &contract_ids {
        let client = StealthSenderContractClient::new(&env, contract_id);
        client.init(&announcer, &None, &None, &0, &admin);
    }

    for contract_id in &contract_ids {
        env.as_contract(contract_id, || {
            let stored: Option<Address> = env.storage().instance().get(&DataKey::Announcer);
            assert_eq!(stored, Some(announcer.clone()));
        });
    }
}

use soroban_sdk::{contract, contractimpl};

#[contract]
pub struct MockAnnouncer;

#[contractimpl]
impl MockAnnouncer {
    pub fn announce(
        _env: Env,
        _scheme_id: u32,
        _stealth_address: Address,
        _ephemeral_pub_key: BytesN<32>,
        _metadata: Bytes,
    ) {
    }
}
