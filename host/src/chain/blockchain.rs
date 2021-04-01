use std::convert::TryInto;
use std::pin::Pin;
use std::str::FromStr;

use async_trait::async_trait;
use destream::{de, en};
use futures::future::TryFutureExt;
use futures::join;
use futures::stream::{self, Stream, StreamExt, TryStreamExt};

use tc_error::*;
use tc_transact::fs::{BlockData, Dir, File, Persist};
use tc_transact::lock::{Mutable, TxnLock};
use tc_transact::{IntoView, Transact};
use tcgeneric::TCPathBuf;

use crate::fs;
use crate::scalar::{Link, Scalar, Value};
use crate::txn::{Txn, TxnId};

use super::{ChainBlock, ChainInstance, ChainType, Schema, Subject, CHAIN, NULL_HASH};
use crate::transact::Transaction;

const BLOCK_SIZE: u64 = 1_000_000;

#[derive(Clone)]
pub struct BlockChain {
    schema: Schema,
    subject: Subject,
    latest: TxnLock<Mutable<u64>>,
    file: fs::File<ChainBlock>,
}

impl BlockChain {
    fn new(schema: Schema, subject: Subject, latest: u64, file: fs::File<ChainBlock>) -> Self {
        Self {
            schema,
            subject,
            latest: TxnLock::new("latest BlockChain block ordinal", latest.into()),
            file,
        }
    }
}

#[async_trait]
impl ChainInstance for BlockChain {
    async fn append(
        &self,
        txn_id: TxnId,
        path: TCPathBuf,
        key: Value,
        value: Scalar,
    ) -> TCResult<()> {
        let latest = self.latest.read(&txn_id).await?;
        let mut block = self.file.write_block(txn_id, (*latest).into()).await?;

        block.append(txn_id, path, key, value);
        Ok(())
    }

    fn subject(&self) -> &Subject {
        &self.subject
    }

    async fn replicate(&self, _txn: &Txn, _source: Link) -> TCResult<()> {
        Err(TCError::not_implemented("BlockChain::replicate"))
    }
}

#[async_trait]
impl Persist for BlockChain {
    type Schema = Schema;
    type Store = fs::Dir;

    fn schema(&self) -> &Schema {
        &self.schema
    }

    async fn load(schema: Schema, dir: fs::Dir, txn_id: TxnId) -> TCResult<Self> {
        let subject = Subject::load(&schema, &dir, txn_id).await?;
        let mut latest = 0;

        let file = if let Some(file) = dir.get_file(&txn_id, &CHAIN.into()).await? {
            // TODO: validate file contents
            let file: fs::File<ChainBlock> = file.try_into()?;

            for block_id in file.block_ids(&txn_id).await? {
                let block_id = u64::from_str(block_id.as_str()).map_err(|e| {
                    TCError::bad_request("blockchain block ID must be a positive integer", e)
                })?;

                if block_id > latest {
                    latest = block_id;
                }
            }

            file
        } else {
            let file = dir
                .create_file(txn_id, CHAIN.into(), ChainType::Sync.into())
                .await?;

            let file: fs::File<ChainBlock> = file.try_into()?;
            if !file.contains_block(&txn_id, &latest.into()).await? {
                file.create_block(txn_id, latest.into(), ChainBlock::new(NULL_HASH))
                    .await?;
            }

            file
        };

        Ok(BlockChain::new(schema, subject, latest, file))
    }
}

#[async_trait]
impl Transact for BlockChain {
    async fn commit(&self, txn_id: &TxnId) {
        {
            let latest = self.latest.read(txn_id).await.expect("latest block number");

            let block = self
                .file
                .read_block(txn_id, &(*latest).into())
                .await
                .expect("read latest chain block");

            if block.size().await.expect("block size") >= BLOCK_SIZE {
                let mut latest = latest.upgrade().await.expect("latest block number");
                (*latest) += 1;

                let hash = block.hash().await.expect("block hash");

                self.file
                    .create_block(*txn_id, (*latest).into(), ChainBlock::new(hash))
                    .await
                    .expect("bump chain block number");
            }
        }

        join!(
            self.latest.commit(txn_id),
            self.subject.commit(txn_id),
            self.file.commit(txn_id)
        );
    }

    async fn finalize(&self, txn_id: &TxnId) {
        join!(
            self.latest.finalize(txn_id),
            self.subject.commit(txn_id),
            self.file.finalize(txn_id)
        );
    }
}

struct ChainVisitor {
    txn: Txn,
}

#[async_trait]
impl de::Visitor for ChainVisitor {
    type Value = BlockChain;

    fn expecting() -> &'static str {
        "a BlockChain"
    }

    async fn visit_seq<A: de::SeqAccess>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let txn_id = *self.txn.id();
        let schema = seq
            .next_element(())
            .await?
            .ok_or_else(|| de::Error::invalid_length(0, "a BlockChain schema"))?;

        let file = self
            .txn
            .context()
            .create_file(txn_id, CHAIN.into(), ChainType::Block.into())
            .map_err(de::Error::custom)
            .await?;
        let file: fs::File<ChainBlock> = file.try_into().map_err(de::Error::custom)?;
        let _file: fs::File<ChainBlock> = seq
            .next_element((txn_id, file))
            .await?
            .ok_or_else(|| de::Error::invalid_length(1, "a BlockChain file"))?;

        BlockChain::load(schema, self.txn.into_context().clone(), txn_id)
            .map_err(de::Error::custom)
            .await
    }
}

#[async_trait]
impl de::FromStream for BlockChain {
    type Context = Txn;

    async fn from_stream<D: de::Decoder>(txn: Txn, decoder: &mut D) -> Result<Self, D::Error> {
        let visitor = ChainVisitor { txn };
        decoder.decode_seq(visitor).await
    }
}

pub type BlockStream = Pin<Box<dyn Stream<Item = TCResult<ChainBlock>> + Send>>;
pub type BlockSeq = en::SeqStream<TCError, ChainBlock, BlockStream>;

#[async_trait]
impl<'en> IntoView<'en, fs::Dir> for BlockChain {
    type Txn = Txn;
    type View = (Schema, BlockSeq);

    async fn into_view(self, txn: Self::Txn) -> TCResult<Self::View> {
        let txn_id = *txn.id();
        let file = self.file;
        let latest = self.latest.read(txn.id()).await?;
        let blocks = stream::iter(0..(*latest))
            .then(move |i| file.clone().read_block_owned(txn_id, i.into()))
            .map_ok(|block| (*block).clone());

        let blocks: BlockStream = Box::pin(blocks);
        let blocks: BlockSeq = en::SeqStream::from(blocks);
        Ok((self.schema, blocks))
    }
}
