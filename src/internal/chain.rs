use std::sync::Arc;

use bytes::Bytes;
use futures::future;
use futures::stream::{FuturesOrdered, Stream};
use futures::{Future, FutureExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::internal::FsDir;
use crate::transaction::TransactionId;

#[derive(Debug, Hash)]
pub struct Chain {
    fs_dir: Arc<FsDir>,
    latest_block: u64,
}

impl Chain {
    pub fn new(fs_dir: Arc<FsDir>) -> Arc<Chain> {
        Arc::new(Chain {
            fs_dir,
            latest_block: 0,
        })
    }

    pub fn from(
        stream: impl Stream<Item = Vec<Bytes>> + Unpin,
        dest: Arc<FsDir>,
    ) -> impl Future<Output = Arc<Chain>> {
        stream
            .fold((0u64, dest), |(i, dest), block| async move {
                dest.clone().flush(i.into(), &block[0], &block[1..]).await;
                (i, dest)
            })
            .then(|(i, dest)| async move {
                Arc::new(Chain {
                    fs_dir: dest,
                    latest_block: i,
                })
            })
    }

    pub fn until<T: 'static + Clone + DeserializeOwned>(
        self: &Arc<Self>,
        txn_id: TransactionId,
    ) -> impl Stream<Item = Vec<T>> {
        self.into_stream()
            .map(move |block: Vec<Bytes>| {
                println!("{}", block.len());
                block
                    .iter()
                    .map(|entry| serde_json::from_slice::<(TransactionId, Vec<T>)>(&entry).unwrap())
                    .filter(|(time, _)| time <= &txn_id)
                    .collect()
            })
            .map(|block: Vec<(TransactionId, Vec<T>)>| {
                let mut result = vec![];
                for (_time, mutations) in block {
                    result.extend(mutations.to_vec())
                }
                result.to_vec()
            })
            .take_while(|block| future::ready(!block.is_empty()))
    }

    pub fn put<T: Serialize>(
        self: Arc<Self>,
        txn_id: &TransactionId,
        mutations: &[T],
    ) -> impl Future<Output = ()> {
        let delta: Vec<Bytes> = mutations
            .iter()
            .map(|e| Bytes::from(serde_json::to_string_pretty(e).unwrap()))
            .collect();
        self.fs_dir
            .clone()
            .flush(self.latest_block.into(), &txn_id.into(), &delta)
    }

    pub fn into_stream(self: &Arc<Self>) -> impl Stream<Item = Vec<Bytes>> {
        let mut stream: FuturesOrdered<Box<dyn Future<Output = Vec<Bytes>> + Unpin + Send>> =
            FuturesOrdered::new();

        for i in 0..self.latest_block {
            let fut = self.fs_dir.clone().get(i.into());
            stream.push(Box::new(fut.boxed()));
        }

        stream
    }
}
