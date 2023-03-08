pub mod geyser {
    tonic::include_proto!("geyser");
}

pub mod solana {
    pub mod storage {
        pub mod confirmed_block {
            tonic::include_proto!("solana.storage.confirmed_block");
        }
    }
}

pub use geyser::*;
pub use solana::storage::confirmed_block::*;

mod convert {
    use {
        solana_sdk::{
            clock::UnixTimestamp,
            instruction::CompiledInstruction,
            message::{
                v0::{LoadedMessage, MessageAddressTableLookup},
                LegacyMessage, MessageHeader, SanitizedMessage,
            },
            pubkey::Pubkey,
            signature::Signature,
            transaction::SanitizedTransaction,
            transaction_context::TransactionReturnData,
        },
        solana_transaction_status::{
            InnerInstruction, InnerInstructions, Reward, RewardType, TransactionStatusMeta,
            TransactionTokenBalance,
        },
    };

    impl From<&SanitizedTransaction> for super::Transaction {
        fn from(value: &SanitizedTransaction) -> Self {
            Self {
                signatures: value
                    .signatures()
                    .iter()
                    .map(|signature| <Signature as AsRef<[u8]>>::as_ref(signature).into())
                    .collect(),
                message: Some(value.message().into()),
            }
        }
    }

    impl From<&SanitizedMessage> for super::Message {
        fn from(message: &SanitizedMessage) -> Self {
            match message {
                SanitizedMessage::Legacy(LegacyMessage { message, .. }) => Self {
                    header: Some((&message.header).into()),
                    account_keys: message
                        .account_keys
                        .iter()
                        .map(|key| <Pubkey as AsRef<[u8]>>::as_ref(key).into())
                        .collect(),
                    recent_blockhash: message.recent_blockhash.to_bytes().into(),
                    instructions: message.instructions.iter().map(|ix| ix.into()).collect(),
                    versioned: false,
                    address_table_lookups: vec![],
                },
                SanitizedMessage::V0(LoadedMessage { message, .. }) => Self {
                    header: Some((&message.header).into()),
                    account_keys: message
                        .account_keys
                        .iter()
                        .map(|key| <Pubkey as AsRef<[u8]>>::as_ref(key).into())
                        .collect(),
                    recent_blockhash: message.recent_blockhash.to_bytes().into(),
                    instructions: message.instructions.iter().map(|ix| ix.into()).collect(),
                    versioned: true,
                    address_table_lookups: message
                        .address_table_lookups
                        .iter()
                        .map(|lookup| lookup.into())
                        .collect(),
                },
            }
        }
    }

    impl From<&MessageHeader> for super::MessageHeader {
        fn from(value: &MessageHeader) -> Self {
            Self {
                num_required_signatures: value.num_required_signatures as u32,
                num_readonly_signed_accounts: value.num_readonly_signed_accounts as u32,
                num_readonly_unsigned_accounts: value.num_readonly_unsigned_accounts as u32,
            }
        }
    }

    impl From<&CompiledInstruction> for super::CompiledInstruction {
        fn from(value: &CompiledInstruction) -> Self {
            Self {
                program_id_index: value.program_id_index as u32,
                accounts: value.accounts.clone(),
                data: value.data.clone(),
            }
        }
    }

    impl From<&MessageAddressTableLookup> for super::MessageAddressTableLookup {
        fn from(lookup: &MessageAddressTableLookup) -> Self {
            Self {
                account_key: <Pubkey as AsRef<[u8]>>::as_ref(&lookup.account_key).into(),
                writable_indexes: lookup.writable_indexes.clone(),
                readonly_indexes: lookup.readonly_indexes.clone(),
            }
        }
    }

    impl From<&TransactionStatusMeta> for super::TransactionStatusMeta {
        fn from(value: &TransactionStatusMeta) -> Self {
            let TransactionStatusMeta {
                status,
                fee,
                pre_balances,
                post_balances,
                inner_instructions,
                log_messages,
                pre_token_balances,
                post_token_balances,
                rewards,
                loaded_addresses,
                return_data,
                compute_units_consumed,
            } = value;
            let err = match status {
                Ok(()) => None,
                Err(err) => Some(super::TransactionError {
                    err: bincode::serialize(&err).expect("transaction error to serialize to bytes"),
                }),
            };
            let inner_instructions_none = inner_instructions.is_none();
            let inner_instructions = inner_instructions
                .as_ref()
                .map(|v| v.iter().map(|ii| ii.into()).collect())
                .unwrap_or_default();
            let log_messages_none = log_messages.is_none();
            let log_messages = log_messages.clone().unwrap_or_default();
            let pre_token_balances = pre_token_balances
                .as_ref()
                .map(|v| v.iter().map(|balance| balance.into()).collect())
                .unwrap_or_default();
            let post_token_balances = post_token_balances
                .as_ref()
                .map(|v| v.iter().map(|balance| balance.into()).collect())
                .unwrap_or_default();
            let rewards = rewards
                .as_ref()
                .map(|v| v.iter().map(|reward| reward.into()).collect())
                .unwrap_or_default();
            let loaded_writable_addresses = loaded_addresses
                .writable
                .iter()
                .map(|key| <Pubkey as AsRef<[u8]>>::as_ref(key).into())
                .collect();
            let loaded_readonly_addresses = loaded_addresses
                .readonly
                .iter()
                .map(|key| <Pubkey as AsRef<[u8]>>::as_ref(key).into())
                .collect();

            Self {
                err,
                fee: *fee,
                pre_balances: pre_balances.clone(),
                post_balances: post_balances.clone(),
                inner_instructions,
                inner_instructions_none,
                log_messages,
                log_messages_none,
                pre_token_balances,
                post_token_balances,
                rewards,
                loaded_writable_addresses,
                loaded_readonly_addresses,
                return_data: return_data.as_ref().map(|d| d.into()),
                return_data_none: return_data.is_none(),
                compute_units_consumed: *compute_units_consumed,
            }
        }
    }

    impl From<&InnerInstructions> for super::InnerInstructions {
        fn from(value: &InnerInstructions) -> Self {
            Self {
                index: value.index as u32,
                instructions: value.instructions.iter().map(|i| i.into()).collect(),
            }
        }
    }

    impl From<&InnerInstruction> for super::InnerInstruction {
        fn from(value: &InnerInstruction) -> Self {
            Self {
                program_id_index: value.instruction.program_id_index as u32,
                accounts: value.instruction.accounts.clone(),
                data: value.instruction.data.clone(),
                stack_height: value.stack_height,
            }
        }
    }

    impl From<&TransactionTokenBalance> for super::TokenBalance {
        fn from(value: &TransactionTokenBalance) -> Self {
            Self {
                account_index: value.account_index as u32,
                mint: value.mint.clone(),
                ui_token_amount: Some(super::UiTokenAmount {
                    ui_amount: value.ui_token_amount.ui_amount.unwrap_or_default(),
                    decimals: value.ui_token_amount.decimals as u32,
                    amount: value.ui_token_amount.amount.clone(),
                    ui_amount_string: value.ui_token_amount.ui_amount_string.clone(),
                }),
                owner: value.owner.clone(),
                program_id: value.program_id.clone(),
            }
        }
    }

    impl From<&Reward> for super::Reward {
        fn from(reward: &Reward) -> Self {
            Self {
                pubkey: reward.pubkey.clone(),
                lamports: reward.lamports,
                post_balance: reward.post_balance,
                reward_type: match reward.reward_type {
                    None => super::RewardType::Unspecified,
                    Some(RewardType::Fee) => super::RewardType::Fee,
                    Some(RewardType::Rent) => super::RewardType::Rent,
                    Some(RewardType::Staking) => super::RewardType::Staking,
                    Some(RewardType::Voting) => super::RewardType::Voting,
                } as i32,
                commission: reward.commission.map(|c| c.to_string()).unwrap_or_default(),
            }
        }
    }

    impl From<&TransactionReturnData> for super::ReturnData {
        fn from(return_data: &TransactionReturnData) -> Self {
            Self {
                program_id: return_data.program_id.to_bytes().into(),
                data: return_data.data.clone(),
            }
        }
    }

    impl From<&[Reward]> for super::Rewards {
        fn from(rewards: &[Reward]) -> Self {
            Self {
                rewards: rewards.iter().map(|v| v.into()).collect(),
            }
        }
    }

    impl From<u64> for super::BlockHeight {
        fn from(block_height: u64) -> Self {
            Self { block_height }
        }
    }

    impl From<UnixTimestamp> for super::UnixTimestamp {
        fn from(timestamp: UnixTimestamp) -> Self {
            Self { timestamp }
        }
    }
}
