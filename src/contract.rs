use std::env;

use crate::error::ContractError;
use crate::msg::{ArbiterResponse, ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{config, config_read, State};
#[cfg(not(feature = "library"))]
use cosmwasm_std::{
    entry_point, to_binary, Addr, BankMsg, Binary, Coin, Deps, DepsMut, Env, MessageInfo, Response,
    StdResult,
};

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:project-name";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let state = State {
        arbiter: deps.api.addr_validate(&msg.arbiter)?,
        recipient: deps.api.addr_validate(&msg.recipient)?,
        source: info.sender,
        end_height: msg.end_height,
        end_time: msg.end_time,
    };
    if state.is_expired(&env) {
        return Err(ContractError::Expired {
            end_height: msg.end_height,
            end_time: msg.end_time,
        });
    }
    config(deps.storage).save(&state)?;

    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    let state = config(deps.storage).load()?;
    match msg {
        ExecuteMsg::Approve { quantity } => try_approve(deps, env, info, state, quantity),
        ExecuteMsg::Refund {} => try_refund(deps, info, env, state),
    }
}

pub fn try_approve(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    state: State,
    quantity: Option<Vec<Coin>>,
) -> Result<Response, ContractError> {
    if state.is_expired(&env) {
        return Err(ContractError::Expired {
            end_height: state.end_height,
            end_time: state.end_time,
        });
    }
    if info.sender != state.arbiter {
        return Err(ContractError::Unauthorized {});
    }
    let amount = if let Some(quantity) = quantity {
        quantity
    } else {
        deps.querier.query_all_balances(&env.contract.address)?
    };
    let res = Response::new()
        .add_message(BankMsg::Send {
            to_address: state.recipient.into_string(),
            amount,
        })
        .add_attribute("Approved", "amount");
    Ok(res)
}
pub fn try_refund(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    state: State,
) -> Result<Response, ContractError> {
    if info.sender != state.arbiter {
        return Err(ContractError::Unauthorized {});
    }
    if !state.is_expired(&env) {
        return Err(ContractError::NotExpired {
            end_height: state.end_height,
            end_time: state.end_time,
        });
    }

    let amount = deps.querier.query_all_balances(&env.contract.address)?;

    let res = Response::new().add_message(BankMsg::Send {
        to_address: state.source.into_string(),
        amount,
    });
    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Arbiter {} => to_binary(&query_arbiter(deps)?),
    }
}

fn query_arbiter(deps: Deps) -> StdResult<ArbiterResponse> {
    let state = config_read(deps.storage).load()?;
    let addr = state.arbiter;
    Ok(ArbiterResponse { arbiter: addr })
}

#[cfg(test)]
mod tests {
    use std::iter::Inspect;

    use crate::msg;

    use super::*;
    use cosmwasm_std::testing::{
        mock_dependencies, mock_dependencies_with_balance, mock_env, mock_info,
    };
    use cosmwasm_std::{coin, coins, from_binary, CosmosMsg, Timestamp};

    fn init_msg_expire_by_height(height: u64) -> InstantiateMsg {
        InstantiateMsg {
            arbiter: String::from("verifies"),
            recipient: String::from("benefits"),
            end_height: Some(height),
            end_time: None,
        }
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies();
        let msg = init_msg_expire_by_height(1000);
        let mut env = mock_env();
        env.block.height = 878;
        env.block.time = Timestamp::from_seconds(0);

        let info = mock_info("creator", &coins(1000, "earth"));

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // query state
        let state = config_read(&mut deps.storage).load().unwrap();
        assert_eq!(
            state,
            State {
                arbiter: Addr::unchecked("verifies"),
                recipient: Addr::unchecked("benefits"),
                source: Addr::unchecked("creator"),
                end_height: Some(1000),
                end_time: None
            }
        );
    }

    #[test]
    fn cannot_initialize_expired() {
        let mut deps = mock_dependencies();
        let msg = init_msg_expire_by_height(1000);
        let mut env = mock_env();
        env.block.height = 1200;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("creator", &coins(1000, "earth"));

        let err = instantiate(deps.as_mut(), env, info, msg).unwrap_err();
        match err {
            ContractError::Expired { .. } => {}
            e => panic!("Unexpected error: {:?}", e),
        };
    }

    #[test]
    fn init_query() {
        let mut deps = mock_dependencies();

        let arbiter = Addr::unchecked("arbiter");
        let recipient = Addr::unchecked("recipient");
        let creator = Addr::unchecked("creator");

        let msg = InstantiateMsg {
            arbiter: arbiter.clone().into(),
            recipient: recipient.into(),
            end_height: None,
            end_time: None,
        };
        let mut env = mock_env();
        env.block.height = 978;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info(creator.as_str(), &[]);

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        let query_response = query_arbiter(deps.as_ref()).unwrap();
        assert_eq!(query_response.arbiter, arbiter);
    }

    #[test]

    fn execute_approve() {
        let mut deps = mock_dependencies();
        let mut env = mock_env();
        env.block.height = 999;
        env.block.time = Timestamp::from_seconds(0);

        let init_amount = coins(1000, "earth");
        let msg = init_msg_expire_by_height(1000);

        let info = mock_info("creator", &init_amount);
        let contract_addr = env.clone().contract.address;

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // update balance
        deps.querier.update_balance(&contract_addr, init_amount);

        // benificiary cannot release it

        let msg = ExecuteMsg::Approve { quantity: None };
        let mut env = mock_env();
        env.block.height = 200;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("beneficiary", &[]);

        let err = execute(deps.as_mut(), env, info, msg.clone()).unwrap_err();
        match err {
            ContractError::Unauthorized { .. } => {}
            e => panic!("Unexpected error: {:?}", e),
        }

        // verifier cannot release it when expired
        let mut env = mock_env();
        env.block.height = 1100;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("verifies", &[]);
        let err = execute(deps.as_mut(), env, info, msg.clone()).unwrap_err();
        match err {
            ContractError::Expired { .. } => {}
            e => panic!("Unexpected error: {}", e),
        }

        // complete release by verifier before expiration
        let mut env = mock_env();
        env.block.height = 900;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("verifies", &[]);

        let res = execute(deps.as_mut(), env, info, msg.clone()).unwrap();
        assert_eq!(1, res.messages.len());
        let msg = res.messages.get(0).expect("no message");
        assert_eq!(
            msg.msg,
            CosmosMsg::Bank(BankMsg::Send {
                to_address: "benifits".into(),
                amount: coins(1000, "earth")
            })
        );

        // partial release by verifier before expiration
        let partial_msg = ExecuteMsg::Approve {
            quantity: Some(coins(500, "earth")),
        };
        let mut env = mock_env();
        env.block.height = 988;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("verifies", &[]);

        let res = execute(deps.as_mut(), env, info, partial_msg).unwrap();
        assert_eq!(1, res.messages.len());

        let msg = res.messages.get(0).expect("no message");
        assert_eq!(
            msg.msg,
            CosmosMsg::Bank(BankMsg::Send {
                to_address: "benificiary".into(),
                amount: coins(500, "earth")
            })
        );
    }

    #[test]
    fn handle_refund() {
        let mut deps = mock_dependencies();
        let init_amount = coins(1000, "earth");
        let msg = init_msg_expire_by_height(1000);
        let mut env = mock_env();
        env.block.height = 900;
        env.block.time = Timestamp::from_seconds(0);

        let contract_addr = env.clone().contract.address;

        let info = mock_info("creator", &init_amount);

        let res = instantiate(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        deps.querier.update_balance(contract_addr, &init_amount);

        //cannot release when unexpired (height < end_height)

        let msg = ExecuteMsg::Refund {};
        let mut env = mock_env();
        env.block.height = 900;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("anybody", &[]);

        let err = execute(deps.as_mut(), env, info, msg).unwrap_err();
        match err {
            ContractError::NotExpired { .. } => {}
            e => panic!("Unexpected error: {}", e),
        };

        // cannot release when unexpired

        // anyone can release
        let msg = ExecuteMsg::Refund {};
        let mut env = mock_env();
        env.block.height = 1001;
        env.block.time = Timestamp::from_seconds(0);
        let info = mock_info("anyone", &[]);

        let res = execute(deps.as_mut(), env, info, msg).unwrap();
        assert_eq!(1, res.messages.len());
        let msg = res.messages.get(0).expect("no messages");
        assert_eq!(
            msg.msg,
            CosmosMsg::Bank(BankMsg::Send {
                to_address: "creator".into(),
                amount: coins(1000, "earth")
            })
        );
    }
}
