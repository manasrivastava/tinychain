use std::convert::TryInto;
use std::sync::Arc;

use async_trait::async_trait;

use crate::transaction::lock::{Mutable, TxnLock};
use crate::transaction::{Transact, Txn, TxnId};
use crate::value::class::NumberType;
use crate::value::{Number, TCResult, UInt, Value};

use super::table;
use super::tensor;

pub struct Graph {
    nodes: table::TableBase,
    edges: table::TableBase,
    max_id: TxnLock<Mutable<u64>>,
}

impl Graph {
    pub async fn create(txn: Arc<Txn>, node_schema: Vec<table::Column>) -> TCResult<Graph> {
        let key: Vec<table::Column> = vec![("id", NumberType::uint64()).try_into()?];
        let nodes = table::Table::create(txn.clone(), (key, node_schema).into()).await?;

        let max_id = 0u64;
        let shape: tensor::Shape = vec![max_id, max_id].into();
        let edges =
            tensor::SparseTable::create_table(txn.clone(), shape.len(), NumberType::uint64())
                .await?;
        let max_id = TxnLock::new(txn.id().clone(), 0u64.into());

        Ok(Graph {
            nodes,
            edges,
            max_id,
        })
    }

    async fn add_node(&self, txn_id: TxnId, node: Vec<Value>) -> TCResult<()> {
        let mut max_id = self.max_id.write(txn_id.clone()).await?;
        self.nodes
            .insert(txn_id, vec![u64_value(&max_id)], node)
            .await?;
        *max_id += 1;
        Ok(())
    }
}

#[async_trait]
impl Transact for Graph {
    async fn commit(&self, txn_id: &TxnId) {
        self.max_id.commit(txn_id).await
    }

    async fn rollback(&self, txn_id: &TxnId) {
        self.max_id.rollback(txn_id).await
    }
}

fn u64_value(value: &u64) -> Value {
    Value::Number(Number::UInt(UInt::U64(*value)))
}
