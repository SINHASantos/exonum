// Copyright 2017 The Exonum Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::str;
use std::num::ParseIntError;

use params::{Map as ParamsMap, Params, Value};
use router::Router;
use iron::prelude::*;
use bodyparser;
use exonum::api::{Api, ApiError};
use exonum::crypto::{CryptoHash, PublicKey, SecretKey, Hash};
use exonum::blockchain::{Blockchain, StoredConfiguration, Schema};
use exonum::storage::StorageValue;
use exonum::node::TransactionSend;
use exonum::encoding::serialize::FromHex;
use exonum::encoding::serialize::json::reexport as serde_json;
use exonum::helpers::Height;

use super::{StorageValueConfigProposeData, TxConfigPropose, TxConfigVote, ConfigurationSchema};

pub type ApiResponseVotesInfo = Option<Vec<Option<TxConfigVote>>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponseConfigHashInfo {
    pub hash: Hash,
    pub config: StoredConfiguration,
    pub propose: Option<Hash>,
    pub votes: ApiResponseVotesInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponseProposeHashInfo {
    pub hash: Hash,
    pub propose_data: StorageValueConfigProposeData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponseConfigInfo {
    pub committed_config: Option<StoredConfiguration>,
    pub propose: Option<StorageValueConfigProposeData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponseProposePost {
    pub tx_hash: Hash,
    pub cfg_hash: Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiResponseVotePost {
    pub tx_hash: Hash,
}

#[derive(Clone)]
pub struct PrivateConfigApi<T: TransactionSend + Clone> {
    pub channel: T,
    pub config: (PublicKey, SecretKey),
}

#[derive(Clone)]
pub struct PublicConfigApi {
    pub blockchain: Blockchain,
}

impl PublicConfigApi {
    fn get_config_with_proofs(&self, config: StoredConfiguration) -> ApiResponseConfigHashInfo {
        let propose = ConfigurationSchema::new(&self.blockchain.snapshot())
            .get_propose(&config.hash())
            .map(|p| p.hash());
        let votes = self.get_votes_for_propose(&config.hash());
        ApiResponseConfigHashInfo {
            hash: config.hash(),
            config,
            propose,
            votes,
        }
    }

    fn get_actual_config(&self) -> ApiResponseConfigHashInfo {
        let snapshot = self.blockchain.snapshot();
        let configuration_schema = Schema::new(&snapshot);
        let actual_cfg = configuration_schema.actual_configuration();
        self.get_config_with_proofs(actual_cfg)
    }

    fn get_following_config(&self) -> Option<ApiResponseConfigHashInfo> {
        Schema::new(&self.blockchain.snapshot())
            .following_configuration()
            .map(|following_cfg| self.get_config_with_proofs(following_cfg))
    }

    fn get_config_by_hash(&self, hash: &Hash) -> ApiResponseConfigInfo {
        let snapshot = self.blockchain.snapshot();
        let general_schema = Schema::new(&snapshot);
        let committed_config = general_schema.configs().get(hash);

        let configuration_schema = ConfigurationSchema::new(&snapshot);
        let propose = configuration_schema.propose_data_by_config_hash().get(hash);
        ApiResponseConfigInfo {
            committed_config,
            propose,
        }
    }

    fn get_votes_for_propose(&self, config_hash: &Hash) -> ApiResponseVotesInfo {
        let snapshot = self.blockchain.snapshot();
        let configuration_schema = ConfigurationSchema::new(&snapshot);
        configuration_schema
            .propose_data_by_config_hash()
            .get(config_hash)
            .map(|_| configuration_schema.get_votes(config_hash))
    }

    fn filter_cfg_predicate(
        cfg: &StoredConfiguration,
        previous_cfg_hash_filter: Option<Hash>,
        actual_from_filter: Option<Height>,
    ) -> bool {
        if let Some(prev_ref) = previous_cfg_hash_filter {
            if cfg.previous_cfg_hash != prev_ref {
                return false;
            }
        }
        if let Some(from_height) = actual_from_filter {
            if cfg.actual_from < from_height {
                return false;
            }
        }
        true
    }

    fn get_all_proposes(
        &self,
        previous_cfg_hash_filter: Option<Hash>,
        actual_from_filter: Option<Height>,
    ) -> Vec<ApiResponseProposeHashInfo> {
        let snapshot = self.blockchain.snapshot();
        let configuration_schema = ConfigurationSchema::new(&snapshot);
        let index = configuration_schema.config_hash_by_ordinal();
        let proposes = {
            index
                .into_iter()
                .map(|cfg_hash| {
                    let propose_data = configuration_schema
                        .propose_data_by_config_hash()
                        .get(&cfg_hash)
                        .expect(&format!(
                            "Not found propose for following cfg_hash: {:?}",
                            cfg_hash
                        ));

                    (cfg_hash, propose_data)
                })
                .filter(|&(_, ref propose_data)| {
                    let cfg = <StoredConfiguration as StorageValue>::from_bytes(
                        propose_data.tx_propose().cfg().as_bytes().into(),
                    );
                    PublicConfigApi::filter_cfg_predicate(
                        &cfg,
                        previous_cfg_hash_filter,
                        actual_from_filter,
                    )
                })
                .map(|(cfg_hash, propose_data)| {
                    ApiResponseProposeHashInfo {
                        hash: cfg_hash,
                        propose_data,
                    }
                })
                .collect::<Vec<_>>()
        };
        proposes
    }

    fn get_all_committed(
        &self,
        previous_cfg_hash_filter: Option<Hash>,
        actual_from_filter: Option<Height>,
    ) -> Vec<ApiResponseConfigHashInfo> {
        let snapshot = self.blockchain.snapshot();
        let general_schema = Schema::new(&snapshot);
        let index = general_schema.configs_actual_from();
        let committed_configs = {
            index
                .into_iter()
                .map(|reference| {
                    let config_hash = reference.cfg_hash();
                    general_schema.configs().get(config_hash).expect(&format!(
                        "Config with hash {:?} is absent in configs table",
                        config_hash
                    ))
                })
                .filter(|config| {
                    PublicConfigApi::filter_cfg_predicate(
                        config,
                        previous_cfg_hash_filter,
                        actual_from_filter,
                    )
                })
                .map(|config| self.get_config_with_proofs(config))
                .collect::<Vec<_>>()
        };
        committed_configs
    }

    fn retrieve_params(map: &ParamsMap) -> Result<(Option<Hash>, Option<Height>), ApiError> {
        let actual_from: Option<Height>;
        let previous_cfg_hash: Option<Hash>;
        previous_cfg_hash = match map.find(&["previous_cfg_hash"]) {
            Some(&Value::String(ref hash_string)) => {
                Some(Hash::from_hex(hash_string).map_err(ApiError::FromHex)?)
            }
            _ => None,
        };
        actual_from = match map.find(&["actual_from"]) {
            Some(&Value::String(ref from_str)) => {
                Some(from_str.parse().map(Height).map_err(|e: ParseIntError| {
                    ApiError::IncorrectRequest(Box::new(e))
                })?)
            }
            _ => None,
        };
        Ok((previous_cfg_hash, actual_from))
    }
}

impl<T> PrivateConfigApi<T>
where
    T: TransactionSend + Clone,
{
    fn put_config_propose(
        &self,
        cfg: StoredConfiguration,
    ) -> Result<ApiResponseProposePost, ApiError> {
        let cfg_hash = cfg.hash();
        let config_propose = TxConfigPropose::new(
            &self.config.0,
            str::from_utf8(cfg.into_bytes().as_slice()).unwrap(),
            &self.config.1,
        );
        let tx_hash = config_propose.hash();
        let ch = self.channel.clone();
        ch.send(Box::new(config_propose))?;
        let res = ApiResponseProposePost { tx_hash, cfg_hash };
        Ok(res)
    }

    fn put_config_vote(&self, cfg_hash: &Hash) -> Result<ApiResponseVotePost, ApiError> {
        let config_vote = TxConfigVote::new(&self.config.0, cfg_hash, &self.config.1);
        let tx_hash = config_vote.hash();
        let ch = self.channel.clone();
        ch.send(Box::new(config_vote))?;
        let res = ApiResponseVotePost { tx_hash };
        Ok(res)
    }
}

impl Api for PublicConfigApi {
    fn wire(&self, router: &mut Router) {

        let self_ = self.clone();
        let config_actual = move |_: &mut Request| -> IronResult<Response> {
            let info = self_.get_actual_config();
            self_.ok_response(&serde_json::to_value(info).unwrap())
        };

        let self_ = self.clone();
        let config_following = move |_: &mut Request| -> IronResult<Response> {
            let info = self_.get_following_config();
            self_.ok_response(&serde_json::to_value(info).unwrap())
        };

        let self_ = self.clone();
        let config_by_hash = move |req: &mut Request| -> IronResult<Response> {
            let params = req.extensions.get::<Router>().unwrap();
            match params.find("hash") {
                Some(hash_str) => {
                    let hash = Hash::from_hex(hash_str).map_err(ApiError::from)?;
                    let info = self_.get_config_by_hash(&hash);
                    self_.ok_response(&serde_json::to_value(info).unwrap())
                }
                None => {
                    Err(ApiError::IncorrectRequest(
                        "Required route \
                                           parameter of configuration \
                                           'hash' is missing"
                            .into(),
                    ))?
                }
            }
        };

        let self_ = self.clone();
        let get_votes_for_propose = move |req: &mut Request| -> IronResult<Response> {
            let params = req.extensions.get::<Router>().unwrap();
            match params.find("hash") {
                Some(hash_str) => {
                    let propose_cfg_hash = Hash::from_hex(hash_str).map_err(ApiError::from)?;
                    let info = self_.get_votes_for_propose(&propose_cfg_hash);
                    self_.ok_response(&serde_json::to_value(info).unwrap())
                }
                None => {
                    Err(ApiError::IncorrectRequest(
                        "Required route \
                                           parameter of configuration \
                                           'hash' is missing"
                            .into(),
                    ))?
                }
            }
        };

        let self_ = self.clone();
        let get_all_proposes = move |req: &mut Request| -> IronResult<Response> {
            let map = req.get_ref::<Params>().unwrap();
            let (previous_cfg_hash, actual_from) = PublicConfigApi::retrieve_params(map)?;
            let info = self_.get_all_proposes(previous_cfg_hash, actual_from);
            self_.ok_response(&serde_json::to_value(info).unwrap())
        };

        let self_ = self.clone();
        let get_all_committed = move |req: &mut Request| -> IronResult<Response> {
            let map = req.get_ref::<Params>().unwrap();
            let (previous_cfg_hash, actual_from) = PublicConfigApi::retrieve_params(map)?;
            let info = self_.get_all_committed(previous_cfg_hash, actual_from);
            self_.ok_response(&serde_json::to_value(info).unwrap())
        };
        router.get("/v1/configs/actual", config_actual, "config_actual");
        router.get(
            "/v1/configs/following",
            config_following,
            "config_following",
        );
        router.get("/v1/configs/:hash", config_by_hash, "config_by_hash");
        router.get(
            "/v1/configs/:hash/votes",
            get_votes_for_propose,
            "get_votes_for_propose",
        );
        router.get("/v1/configs/proposed", get_all_proposes, "get_all_proposes");
        router.get(
            "/v1/configs/committed",
            get_all_committed,
            "get_all_committed",
        );

    }
}

impl<T> Api for PrivateConfigApi<T>
where
    T: 'static + TransactionSend + Clone,
{
    fn wire(&self, router: &mut Router) {
        let self_ = self.clone();
        let put_config_propose = move |req: &mut Request| -> IronResult<Response> {
            match req.get::<bodyparser::Struct<StoredConfiguration>>() {
                Ok(Some(cfg)) => {
                    let info = self_.put_config_propose(cfg)?;
                    self_.ok_response(&serde_json::to_value(info).unwrap())
                }
                Ok(None) => Err(ApiError::IncorrectRequest("Empty request body".into()))?,
                Err(e) => Err(ApiError::IncorrectRequest(Box::new(e)))?,
            }
        };

        let self_ = self.clone();
        let put_config_vote = move |req: &mut Request| -> IronResult<Response> {
            let params = req.extensions.get::<Router>().unwrap();
            match params.find("hash") {
                Some(hash_str) => {
                    let propose_cfg_hash = Hash::from_hex(hash_str).map_err(ApiError::from)?;
                    let info = self_.put_config_vote(&propose_cfg_hash)?;
                    self_.ok_response(&serde_json::to_value(info).unwrap())
                }
                None => {
                    Err(ApiError::IncorrectRequest(
                        "Required route \
                                           parameter of configuration \
                                           'hash' is missing"
                            .into(),
                    ))?
                }
            }
        };
        router.post(
            "/v1/configs/postpropose",
            put_config_propose,
            "put_config_propose",
        );
        router.post(
            "/v1/configs/:hash/postvote",
            put_config_vote,
            "put_config_vote",
        );
    }
}
