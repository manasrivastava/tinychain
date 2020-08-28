use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::class::{Instance, State, TCResult};
use crate::error;
use crate::transaction::{Transact, Txn, TxnId};
use crate::value::Value;

pub mod btree;
pub mod class;
pub mod graph;
pub mod schema;
pub mod table;
pub mod tensor;

pub type CollectionType = class::CollectionType;

pub type BTree = btree::BTree;
pub type BTreeSlice = btree::BTreeSlice;
pub type Graph = graph::Graph;
pub type Table = table::Table;
pub type Tensor = tensor::Tensor;

#[derive(Clone)]
pub enum CollectionBase {
    BTree(btree::BTreeFile),
    Graph(graph::Graph),
    Table(table::TableBase),
    Tensor(tensor::TensorBase),
}

impl Instance for CollectionBase {
    type Class = class::CollectionBaseType;

    fn class(&self) -> Self::Class {
        match self {
            Self::BTree(_) => class::CollectionBaseType::BTree,
            Self::Graph(_) => class::CollectionBaseType::Graph, // TODO
            Self::Table(_) => class::CollectionBaseType::Table, // TODO
            Self::Tensor(_) => class::CollectionBaseType::Tensor, // TODO
        }
    }
}

#[async_trait]
impl Transact for CollectionBase {
    async fn commit(&self, txn_id: &TxnId) {
        match self {
            Self::BTree(btree) => btree.commit(txn_id).await,
            Self::Graph(graph) => graph.commit(txn_id).await,
            Self::Table(table) => table.commit(txn_id).await,
            Self::Tensor(tensor) => tensor.commit(txn_id).await,
        }
    }

    async fn rollback(&self, txn_id: &TxnId) {
        match self {
            Self::BTree(btree) => btree.rollback(txn_id).await,
            Self::Graph(graph) => graph.rollback(txn_id).await,
            Self::Table(table) => table.rollback(txn_id).await,
            Self::Tensor(tensor) => tensor.rollback(txn_id).await,
        }
    }
}

impl fmt::Display for CollectionBase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BTree(_) => write!(f, "(B-tree)"),
            Self::Graph(_) => write!(f, "(graph)"),
            Self::Table(_) => write!(f, "(table)"),
            Self::Tensor(_) => write!(f, "(tensor)"),
        }
    }
}

impl CollectionBase {
    pub async fn get(&self, _txn: Arc<Txn>, _selector: Value) -> TCResult<State> {
        Err(error::not_implemented())
    }

    pub async fn put(&self, _txn: &Arc<Txn>, _selector: &Value, _state: State) -> TCResult<Self> {
        Err(error::not_implemented())
    }
}

#[derive(Clone)]
pub enum CollectionView {
    BTree(btree::BTree),
    Graph(graph::Graph),
    Table(table::Table),
    Tensor(tensor::Tensor),
}

impl Instance for CollectionView {
    type Class = class::CollectionViewType;

    fn class(&self) -> Self::Class {
        match self {
            Self::BTree(btree) => btree.class().into(),
            _ => unimplemented!(), // TODO
        }
    }
}

#[async_trait]
impl Transact for CollectionView {
    async fn commit(&self, txn_id: &TxnId) {
        match self {
            Self::BTree(btree) => btree.commit(txn_id).await,
            Self::Graph(graph) => graph.commit(txn_id).await,
            Self::Table(table) => table.commit(txn_id).await,
            Self::Tensor(tensor) => tensor.commit(txn_id).await,
        }
    }

    async fn rollback(&self, txn_id: &TxnId) {
        match self {
            Self::BTree(btree) => btree.rollback(txn_id).await,
            Self::Graph(graph) => graph.rollback(txn_id).await,
            Self::Table(table) => table.rollback(txn_id).await,
            Self::Tensor(tensor) => tensor.rollback(txn_id).await,
        }
    }
}

impl fmt::Display for CollectionView {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BTree(_) => write!(f, "(B-tree view)"),
            Self::Graph(_) => write!(f, "(graph)"),
            Self::Table(_) => write!(f, "(table view)"),
            Self::Tensor(_) => write!(f, "(tensor view)"),
        }
    }
}

#[derive(Clone)]
pub enum Collection {
    Base(CollectionBase),
    View(CollectionView),
}

impl Collection {
    pub async fn get(&self, _txn: Arc<Txn>, _selector: Value) -> TCResult<State> {
        Err(error::not_implemented())
    }

    pub async fn put(&self, _txn: &Arc<Txn>, _selector: &Value, _state: State) -> TCResult<Self> {
        Err(error::not_implemented())
    }
}

impl Instance for Collection {
    type Class = CollectionType;

    fn class(&self) -> CollectionType {
        match self {
            Self::Base(base) => base.class().into(),
            Self::View(view) => view.class().into(),
        }
    }
}

#[async_trait]
impl Transact for Collection {
    async fn commit(&self, txn_id: &TxnId) {
        match self {
            Self::Base(base) => base.commit(txn_id).await,
            Self::View(view) => view.commit(txn_id).await,
        }
    }

    async fn rollback(&self, txn_id: &TxnId) {
        match self {
            Self::Base(base) => base.rollback(txn_id).await,
            Self::View(view) => view.rollback(txn_id).await,
        }
    }
}

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Base(base) => write!(f, "{}", base),
            Self::View(view) => write!(f, "{}", view),
        }
    }
}
