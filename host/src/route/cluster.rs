use std::collections::HashSet;
use std::iter::FromIterator;

use bytes::Bytes;
use futures::future::{self, try_join_all, FutureExt, TryFutureExt};
use futures::stream::{FuturesUnordered, StreamExt};
use log::debug;
use safecast::{TryCastFrom, TryCastInto};

use tc_error::*;
use tc_transact::{Transact, Transaction};
use tcgeneric::{label, Id, TCPath, Tuple};

use crate::cluster::{Cluster, REPLICAS};
use crate::route::*;
use crate::scalar::{Link, Value};
use crate::state::State;
use crate::txn::Txn;

struct AuthorizeHandler<'a> {
    cluster: &'a Cluster,
}

impl<'a> Handler<'a> for AuthorizeHandler<'a> {
    fn get(self: Box<Self>) -> Option<GetHandler<'a>> {
        Some(Box::new(|txn, scope| {
            Box::pin(async move {
                let scope = scope
                    .try_cast_into(|v| TCError::bad_request("expected an auth scope, not", v))?;

                self.cluster
                    .authorize(&txn, &scope)
                    .map_ok(State::from)
                    .await
            })
        }))
    }
}

impl<'a> From<&'a Cluster> for AuthorizeHandler<'a> {
    fn from(cluster: &'a Cluster) -> Self {
        Self { cluster }
    }
}

pub struct ClusterHandler<'a> {
    cluster: &'a Cluster,
}

impl<'a> ClusterHandler<'a> {
    fn handle_get(self, key: Value) -> TCResult<State> {
        debug!("Cluster::get {}", key);

        if key.is_some() {
            let key: Id = key.try_cast_into(|v| TCError::bad_request("invalid ID", v))?;
            self.cluster
                .chain(&key)
                .cloned()
                .map(State::from)
                .ok_or_else(|| TCError::not_found(key))
        } else {
            let public_key = Bytes::from(self.cluster.public_key().to_vec());
            Ok(Value::from(public_key).into())
        }
    }
}

impl<'a> Handler<'a> for ClusterHandler<'a> {
    fn get(self: Box<Self>) -> Option<GetHandler<'a>> {
        Some(Box::new(|_txn, key| {
            Box::pin(future::ready(self.handle_get(key)))
        }))
    }

    fn put(self: Box<Self>) -> Option<PutHandler<'a>> {
        Some(Box::new(|txn, key, value| {
            Box::pin(async move {
                if key.is_some() {
                    return Err(TCError::unsupported("a Cluster itself is immutable"));
                }

                let participant =
                    value.try_cast_into(|s| TCError::bad_request("expected a Link, not", s))?;

                self.cluster.mutate(&txn, participant).await
            })
        }))
    }

    fn post(self: Box<Self>) -> Option<PostHandler<'a>> {
        Some(Box::new(|txn, params| {
            Box::pin(async move {
                // TODO: authorize request using a scope

                if !params.is_empty() {
                    return Err(TCError::bad_request(
                        "unrecognized commit parameters",
                        params,
                    ));
                }

                self.cluster.commit(txn.id()).await;
                Ok(State::default())
            })
        }))
    }
}

impl<'a> From<&'a Cluster> for ClusterHandler<'a> {
    fn from(cluster: &'a Cluster) -> Self {
        Self { cluster }
    }
}

struct GrantHandler<'a> {
    cluster: &'a Cluster,
}

impl<'a> Handler<'a> for GrantHandler<'a> {
    fn post(self: Box<Self>) -> Option<PostHandler<'a>> {
        Some(Box::new(|txn, mut params| {
            Box::pin(async move {
                let scope = params.require(&label("scope").into())?;
                let op = params.require(&label("op").into())?;
                let context = params.or_default(&label("context").into())?;
                params.expect_empty()?;
                self.cluster.grant(txn, scope, op, context).await
            })
        }))
    }
}

impl<'a> From<&'a Cluster> for GrantHandler<'a> {
    fn from(cluster: &'a Cluster) -> Self {
        Self { cluster }
    }
}

struct InstallHandler<'a> {
    cluster: &'a Cluster,
}

impl<'a> Handler<'a> for InstallHandler<'a> {
    fn put(self: Box<Self>) -> Option<PutHandler<'a>> {
        Some(Box::new(|txn, link, scopes| {
            Box::pin(async move {
                let link = link.try_cast_into(|v| {
                    TCError::bad_request("install requires a Link to a Cluster, not", v)
                })?;

                let scopes = Tuple::try_cast_from(scopes, |v| {
                    TCError::bad_request("expected a list of authorization scopes, not", v)
                })?;

                self.cluster
                    .install(*txn.id(), link, scopes.into_iter().collect())
                    .await
            })
        }))
    }
}

impl<'a> From<&'a Cluster> for InstallHandler<'a> {
    fn from(cluster: &'a Cluster) -> Self {
        Self { cluster }
    }
}

struct ReplicaHandler<'a> {
    cluster: &'a Cluster,
}

impl<'a> Handler<'a> for ReplicaHandler<'a> {
    fn get(self: Box<Self>) -> Option<GetHandler<'a>> {
        Some(Box::new(|txn, key| {
            Box::pin(async move {
                key.expect_none()?;

                self.cluster
                    .replicas(txn.id())
                    .map_ok(Value::from_iter)
                    .map_ok(State::from)
                    .await
            })
        }))
    }

    fn put(self: Box<Self>) -> Option<PutHandler<'a>> {
        Some(Box::new(|txn, key, link| {
            Box::pin(async move {
                key.expect_none()?;

                let link = link.try_cast_into(|v| {
                    TCError::bad_request("expected a Link to a Cluster, not", v)
                })?;

                self.cluster.add_replica(&txn, link).await
            })
        }))
    }

    fn delete(self: Box<Self>) -> Option<DeleteHandler<'a>> {
        Some(Box::new(|txn, link| {
            Box::pin(async move {
                let link = link.try_cast_into(|v| {
                    TCError::bad_request("expected a Link to a Cluster, not", v)
                })?;

                self.cluster.remove_replica(*txn.id(), &link).await
            })
        }))
    }
}

impl<'a> From<&'a Cluster> for ReplicaHandler<'a> {
    fn from(cluster: &'a Cluster) -> Self {
        Self { cluster }
    }
}

struct ReplicateHandler<'a> {
    cluster: &'a Cluster,
    path: &'a [PathSegment],
}

impl<'a> ReplicateHandler<'a> {
    fn new(cluster: &'a Cluster, path: &'a [PathSegment]) -> Self {
        Self { cluster, path }
    }

    fn handler(&self) -> Option<Box<dyn Handler<'a> + 'a>> {
        if self.path.is_empty() {
            Some(Box::new(ClusterHandler::from(self.cluster)))
        } else if let Some(chain) = self.cluster.chain(&self.path[0]) {
            chain.route(&self.path[1..])
        } else if let Some(class) = self.cluster.class(&self.path[0]) {
            class.route(&self.path[1..])
        } else if self.path.len() == 1 {
            match self.path[0].as_str() {
                "authorize" => Some(Box::new(AuthorizeHandler::from(self.cluster))),
                "grant" => Some(Box::new(GrantHandler::from(self.cluster))),
                "install" => Some(Box::new(InstallHandler::from(self.cluster))),
                "replicas" => Some(Box::new(ReplicaHandler::from(self.cluster))),
                _ => None,
            }
        } else {
            None
        }
    }

    async fn replicate_write<
        'b,
        F: Future<Output = (Link, TCResult<()>)>,
        W: Fn(&'b Txn, Link) -> F,
    >(
        cluster: &'a Cluster,
        txn: &'b Txn,
        write: W,
    ) -> TCResult<()>
    where
        'a: 'b,
    {
        let replicas = cluster.replicas(txn.id()).await?;
        let max_failures = replicas.len() / 2;
        let mut failed = HashSet::with_capacity(replicas.len());
        let mut succeeded = HashSet::with_capacity(replicas.len());

        {
            let mut results =
                FuturesUnordered::from_iter(replicas.into_iter().map(|link| write(txn, link)));

            while let Some((replica, result)) = results.next().await {
                match result {
                    Err(cause) if cause.code() == ErrorType::Conflict => return Err(cause),
                    Err(_) => failed.insert(replica),
                    Ok(()) => succeeded.insert(replica),
                };

                if failed.len() > max_failures {
                    assert!(result.is_err());
                    return result;
                }
            }
        }

        let failed = Value::from_iter(failed);
        try_join_all(
            succeeded
                .into_iter()
                .map(|replica| txn.delete(replica.append(REPLICAS.into()), failed.clone())),
        )
        .await?;

        Ok(())
    }
}

impl<'a> Handler<'a> for ReplicateHandler<'a> {
    fn get(self: Box<Self>) -> Option<GetHandler<'a>> {
        let handler = self.handler()?.get()?;

        Some(Box::new(|txn, key| {
            Box::pin(async move {
                if txn.is_owner(self.cluster.path()) {
                    handler(txn, key).await
                } else if let Some(owner) = txn.owner() {
                    if &owner.path()[..] == self.cluster.path() {
                        let mut link = owner.clone();
                        link.extend(self.path.to_vec());

                        debug!("route GET request to transaction owner {}", link);
                        txn.get(link, key).await
                    } else {
                        handler(txn, key).await
                    }
                } else {
                    handler(txn, key).await
                }
            })
        }))
    }

    fn put(self: Box<Self>) -> Option<PutHandler<'a>> {
        let handler = self.handler()?.put()?;

        Some(Box::new(|txn, key, value| {
            Box::pin(async move {
                handler(txn.clone(), key.clone(), value.clone()).await?;

                if !txn.is_owner(self.cluster.path()) {
                    return Ok(());
                }

                Self::replicate_write(self.cluster, &txn, |txn, replica_link| {
                    let mut target = replica_link.clone();
                    target.extend(self.path.to_vec());

                    txn.put(target, key.clone(), value.clone())
                        .map(|r| (replica_link, r))
                })
                .await?;

                Ok(())
            })
        }))
    }

    fn post(self: Box<Self>) -> Option<PostHandler<'a>> {
        self.handler()?.post()
    }

    fn delete(self: Box<Self>) -> Option<DeleteHandler<'a>> {
        let handler = self.handler()?.delete()?;

        Some(Box::new(|txn, key| {
            Box::pin(async move {
                handler(txn.clone(), key.clone()).await?;

                if !txn.is_owner(self.cluster.path()) {
                    return Ok(());
                }

                Self::replicate_write(self.cluster, &txn, |txn, replica_link| {
                    let mut target = replica_link.clone();
                    target.extend(self.path.to_vec());

                    txn.delete(target, key.clone()).map(|r| (replica_link, r))
                })
                .await?;

                Ok(())
            })
        }))
    }
}

impl Route for Cluster {
    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<Box<dyn Handler<'a> + 'a>> {
        debug!("Cluster::route {}", TCPath::from(path));

        Some(Box::new(ReplicateHandler::new(self, path)))
    }
}
