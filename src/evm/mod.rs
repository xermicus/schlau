mod runtime;

use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::JsonAbi;
use fp_evm::{CreateInfo, ExitReason};
use frame_support::sp_runtime;
use frame_support::traits::fungible::Mutate;
use frame_system::GenesisConfig;
use pallet_evm::Runner;
use sp_core::{H160, H256, U256};
use sp_io::TestExternalities;
use sp_runtime::BuildStorage;

pub use runtime::EvmRuntime;

pub type AccountIdFor<R> = <R as frame_system::Config>::AccountId;
pub type BalanceOf<R> = <R as pallet_balances::Config>::Balance;

pub const ACCOUNTS: [H160; 8] = [
    H160::repeat_byte(0),
    H160::repeat_byte(1),
    H160::repeat_byte(2),
    H160::repeat_byte(3),
    H160::repeat_byte(4),
    H160::repeat_byte(5),
    H160::repeat_byte(6),
    H160::repeat_byte(7),
];

pub struct EvmContract {
    address: H160,
    abi: JsonAbi,
    pub sandbox: EvmSandbox<EvmRuntime>,
}

impl EvmContract {
    pub fn init(contract: &str) -> Self {
        let result =
            crate::solc::build_contract(&format!("contracts/solidity/{}.sol", contract)).unwrap();
        let mut sandbox = EvmSandbox::<EvmRuntime>::new();

        let create_args = CreateArgs {
            source: ACCOUNTS[0],
            init: result.code,
            gas_limit: 1_000_000_000,
            max_fee_per_gas: U256::from(1_000_000_000),
            ..Default::default()
        };
        let address = sandbox.create(create_args).unwrap();
        EvmContract {
            address,
            abi: result.abi,
            sandbox,
        }
    }

    pub fn call_args(&self, func: &str, args: &[DynSolValue]) -> CallArgs {
        let func = &self.abi.function(func).unwrap()[0];
        let data = func.abi_encode_input(args).unwrap();

        CallArgs {
            source: ACCOUNTS[0],
            target: self.address,
            input: data,
            gas_limit: 1_000_000_000,
            max_fee_per_gas: U256::from(1_000_000_000),
            ..Default::default()
        }
    }
}

pub struct EvmSandbox<R = EvmRuntime> {
    externalities: TestExternalities,
    phantom: std::marker::PhantomData<R>,
}

impl<R> EvmSandbox<R>
where
    R: pallet_evm::Config + pallet_balances::Config + pallet_balances::Config<Balance = u128>,
    AccountIdFor<R>: From<H160> + Into<H160>,
    BalanceOf<R>: From<u128>,
{
    pub fn new() -> Self {
        let mut storage = GenesisConfig::<R>::default()
            .build_storage()
            .expect("error building storage");

        // initialize the balance of the default account
        pallet_balances::GenesisConfig::<R> {
            balances: ACCOUNTS
                .iter()
                .map(|acc| (AccountIdFor::<R>::from(*acc), u64::MAX as u128))
                .collect(),
        }
        .assimilate_storage(&mut storage)
        .unwrap();

        Self {
            externalities: TestExternalities::new(storage),
            phantom: Default::default(),
        }
    }

    pub fn execute_with<T>(&mut self, execute: impl FnOnce() -> T) -> T {
        self.externalities.execute_with(execute)
    }

    pub fn create(&mut self, create_args: CreateArgs) -> anyhow::Result<H160> {
        let CreateArgs {
            source,
            init,
            value,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            nonce,
            access_list,
        } = create_args;
        self.execute_with(|| {
            let is_transactional = true;
            let validate = true;
            let CreateInfo {
                exit_reason,
                value: create_address,
                ..
            } = R::Runner::create(
                source,
                init,
                value,
                gas_limit,
                Some(max_fee_per_gas),
                max_priority_fee_per_gas,
                nonce,
                access_list,
                is_transactional,
                validate,
                None,
                None,
                R::config(),
            )
            .map_err(|err| {
                let err: sp_runtime::DispatchError = err.error.into();
                let ser_err = serde_json::to_string_pretty(&err).unwrap();
                anyhow::anyhow!("error invoking create: {}", ser_err)
            })?;

            if let ExitReason::Succeed(_) = exit_reason {
                Ok(create_address)
            } else {
                Err(anyhow::anyhow!("create failed: {:?}", exit_reason))
            }
        })
    }

    pub fn call(&mut self, call_args: CallArgs) -> anyhow::Result<Vec<u8>> {
        let CallArgs {
            source,
            target,
            input,
            value,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            nonce,
            access_list,
        } = call_args;
        self.execute_with(|| {
            let is_transactional = true;
            let validate = true;
            let info = R::Runner::call(
                source,
                target,
                input,
                value,
                gas_limit,
                Some(max_fee_per_gas),
                max_priority_fee_per_gas,
                nonce,
                access_list,
                is_transactional,
                validate,
                None,
                None,
                R::config(),
            )
            .map_err(|err| {
                let err: sp_runtime::DispatchError = err.error.into();
                let ser_err = serde_json::to_string_pretty(&err).unwrap();
                anyhow::anyhow!("error invoking call: {}", ser_err)
            })?;
            if let ExitReason::Succeed(_) = info.exit_reason {
                Ok(info.value)
            } else {
                Err(anyhow::anyhow!("call failed: {:?}", info.exit_reason))
            }
        })
    }

    pub fn mint_into(
        &mut self,
        address: H160,
        amount: BalanceOf<R>,
    ) -> anyhow::Result<BalanceOf<R>> {
        let address = AccountIdFor::<R>::from(address);
        self.execute_with(|| pallet_balances::Pallet::<R>::mint_into(&address, amount))
            .map_err(|_err| anyhow::anyhow!("error minting into account"))
    }

    /// Return the free balance of an account.
    ///
    /// # Arguments
    ///
    /// * `address` - The address of the account to query.
    pub fn free_balance(&mut self, address: H160) -> BalanceOf<R> {
        let address = AccountIdFor::<R>::from(address);
        self.execute_with(|| pallet_balances::Pallet::<R>::free_balance(&address))
    }
}

#[derive(Default)]
pub struct CreateArgs {
    pub source: H160,
    pub init: Vec<u8>,
    pub value: U256,
    pub gas_limit: u64,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: Option<U256>,
    pub nonce: Option<U256>,
    pub access_list: Vec<(H160, Vec<H256>)>,
}

#[derive(Default, Clone)]

pub struct CallArgs {
    pub source: H160,
    pub target: H160,
    pub input: Vec<u8>,
    pub value: U256,
    pub gas_limit: u64,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: Option<U256>,
    pub nonce: Option<U256>,
    pub access_list: Vec<(H160, Vec<H256>)>,
}
