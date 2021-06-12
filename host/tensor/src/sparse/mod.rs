use std::convert::{TryFrom, TryInto};
use std::marker::PhantomData;
use std::pin::Pin;

use async_trait::async_trait;
use destream::{de, en};
use futures::{Stream, TryFutureExt};

use tc_btree::{BTreeType, Node};
use tc_error::*;
use tc_transact::fs::{Dir, File};
use tc_transact::{IntoView, Transaction};
use tc_value::{Number, NumberType, ValueType};

use super::{Coord, Shape, TensorAccess};

pub use access::{SparseAccess, SparseAccessor};
pub use table::SparseTable;
use tcgeneric::NativeClass;

mod access;
mod table;

pub type SparseRow = (Coord, Number);
pub type SparseStream<'a> = Pin<Box<dyn Stream<Item = TCResult<SparseRow>> + Send + Unpin + 'a>>;

#[derive(Clone)]
pub struct SparseTensor<F: File<Node>, D: Dir, T: Transaction<D>, A: SparseAccess<F, D, T>> {
    accessor: A,
    file: PhantomData<F>,
    dir: PhantomData<D>,
    txn: PhantomData<T>,
}

impl<F: File<Node>, D: Dir, T: Transaction<D>, A: SparseAccess<F, D, T>> SparseTensor<F, D, T, A> {
    pub fn into_inner(self) -> A {
        self.accessor
    }
}

impl<F: File<Node>, D: Dir, T: Transaction<D>, A: SparseAccess<F, D, T>> TensorAccess
    for SparseTensor<F, D, T, A>
{
    fn dtype(&self) -> NumberType {
        self.accessor.dtype()
    }

    fn ndim(&self) -> usize {
        self.accessor.ndim()
    }

    fn shape(&self) -> &Shape {
        self.accessor.shape()
    }

    fn size(&self) -> u64 {
        self.accessor.size()
    }
}

impl<F: File<Node>, D: Dir, T: Transaction<D>, A: SparseAccess<F, D, T>> From<A>
    for SparseTensor<F, D, T, A>
{
    fn from(accessor: A) -> Self {
        Self {
            accessor,
            file: PhantomData,
            dir: PhantomData,
            txn: PhantomData,
        }
    }
}

#[async_trait]
impl<'en, F: File<Node>, D: Dir, T: Transaction<D>, A: SparseAccess<F, D, T>> IntoView<'en, D>
    for SparseTensor<F, D, T, A>
{
    type Txn = T;
    type View = SparseTensorView<'en>;

    async fn into_view(self, txn: Self::Txn) -> TCResult<Self::View> {
        Ok(SparseTensorView {
            shape: self.shape().to_vec(),
            dtype: self.dtype().into(),
            filled: self.accessor.filled(txn).await?,
        })
    }
}

#[async_trait]
impl<F: File<Node>, D: Dir, T: Transaction<D>> de::FromStream
    for SparseTensor<F, D, T, SparseTable<F, D, T>>
where
    F: TryFrom<D::File, Error = TCError>,
    D::FileClass: From<BTreeType>,
{
    type Context = T;

    async fn from_stream<De: de::Decoder>(txn: T, decoder: &mut De) -> Result<Self, De::Error> {
        decoder.decode_seq(SparseTensorVisitor::new(txn)).await
    }
}

struct SparseTensorVisitor<F: File<Node>, D: Dir, T: Transaction<D>> {
    txn: T,
    file: PhantomData<F>,
    dir: PhantomData<D>,
}

impl<F: File<Node>, D: Dir, T: Transaction<D>> SparseTensorVisitor<F, D, T> {
    fn new(txn: T) -> Self {
        Self {
            txn,
            file: PhantomData,
            dir: PhantomData,
        }
    }
}

#[async_trait]
impl<F: File<Node>, D: Dir, T: Transaction<D>> de::Visitor for SparseTensorVisitor<F, D, T>
where
    F: TryFrom<D::File, Error = TCError>,
    D::FileClass: From<BTreeType>,
{
    type Value = SparseTensor<F, D, T, SparseTable<F, D, T>>;

    fn expecting() -> &'static str {
        "a SparseTensor"
    }

    async fn visit_seq<A: de::SeqAccess>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let schema = seq.next_element::<(Vec<u64>, ValueType)>(()).await?;
        let (shape, dtype) = schema.ok_or_else(|| de::Error::invalid_length(0, "tensor schema"))?;
        let dtype = dtype.try_into().map_err(de::Error::custom)?;

        let txn_id = *self.txn.id();
        let table = SparseTable::create(self.txn.context(), txn_id, shape.into(), dtype)
            .map_err(de::Error::custom)
            .await?;

        if let Some(table) = seq
            .next_element::<SparseTable<F, D, T>>((table, txn_id))
            .await?
        {
            Ok(SparseTensor::from(table))
        } else {
            Err(de::Error::custom("invalid SparseTensor"))
        }
    }
}

pub struct SparseTensorView<'en> {
    shape: Vec<u64>,
    dtype: ValueType,
    filled: SparseStream<'en>,
}

impl<'en> en::IntoStream<'en> for SparseTensorView<'en> {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let schema = (self.shape.to_vec(), self.dtype.path());
        let filled = en::SeqStream::from(self.filled);
        (schema, filled).into_stream(encoder)
    }
}