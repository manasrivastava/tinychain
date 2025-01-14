use std::convert::TryFrom;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Add, Div, Mul, Sub};
use std::pin::Pin;

use afarray::Array;
use async_trait::async_trait;
use destream::{de, en};
use futures::future::{self, TryFutureExt};
use futures::stream::{self, Stream, StreamExt, TryStreamExt};
use log::debug;

use tc_btree::{BTreeType, Node};
use tc_error::*;
use tc_transact::fs::{CopyFrom, Dir, File, Hash, Persist, Restore};
use tc_transact::{IntoView, Transact, Transaction, TxnId};
use tc_value::{Number, NumberClass, NumberInstance, NumberType};
use tcgeneric::{TCBoxTryFuture, TCBoxTryStream};

use super::dense::{BlockListSparse, DenseTensor};
use super::{
    Bounds, Coord, Phantom, Schema, Shape, Tensor, TensorAccess, TensorBoolean, TensorCompare,
    TensorDualIO, TensorIO, TensorInstance, TensorMath, TensorReduce, TensorTransform, TensorType,
    TensorUnary,
};

use crate::dense::PER_BLOCK;

use access::*;
pub use access::{DenseToSparse, SparseAccess, SparseAccessor};
pub use table::SparseTable;

mod access;
mod combine;
mod table;

pub type SparseRow = (Coord, Number);
pub type SparseStream<'a> = Pin<Box<dyn Stream<Item = TCResult<SparseRow>> + Send + Unpin + 'a>>;

const ERR_NOT_SPARSE: &str = "The result of the requested operation would not be sparse;\
convert to a DenseTensor first.";

/// A `Tensor` stored as a `Table` of [`Coord`]s and [`Number`] values
#[derive(Clone)]
pub struct SparseTensor<FD, FS, D, T, A> {
    accessor: A,
    phantom: Phantom<FD, FS, D, T>,
}

impl<FD, FS, D, T, A> SparseTensor<FD, FS, D, T, A> {
    /// Consume this [`SparseTensor`] and return its accessor.
    pub fn into_inner(self) -> A {
        self.accessor
    }
}

type Condensed<FD, FS, D, T, L, R> =
    DenseTensor<FD, FS, D, T, BlockListSparse<FD, FS, D, T, SparseCombinator<FD, FS, D, T, L, R>>>;

impl<FD, FS, D, T, A> SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
{
    fn combine<R: SparseAccess<FD, FS, D, T>>(
        self,
        other: SparseTensor<FD, FS, D, T, R>,
        combinator: fn(Number, Number) -> Number,
        dtype: NumberType,
    ) -> TCResult<SparseTensor<FD, FS, D, T, SparseCombinator<FD, FS, D, T, A, R>>> {
        if self.shape() != other.shape() {
            return Err(TCError::unsupported(format!(
                "cannot compare Tensors of different shapes: {}, {}",
                self.shape(),
                other.shape()
            )));
        }

        let accessor = SparseCombinator::new(self.accessor, other.accessor, combinator, dtype)?;

        Ok(SparseTensor {
            accessor,
            phantom: self.phantom,
        })
    }

    fn condense<R>(
        self,
        other: SparseTensor<FD, FS, D, T, R>,
        condensor: fn(Number, Number) -> Number,
    ) -> TCResult<Condensed<FD, FS, D, T, A, R>>
    where
        R: SparseAccess<FD, FS, D, T>,
    {
        if self.shape() != other.shape() {
            return Err(TCError::unsupported(format!(
                "cannot condense sparse Tensor of size {} with another of size {}",
                self.shape(),
                other.shape()
            )));
        }

        let accessor =
            SparseCombinator::new(self.accessor, other.accessor, condensor, NumberType::Bool)?;

        let dense = BlockListSparse::from(accessor);
        Ok(dense.into())
    }
}

impl<FD, FS, D, T> SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    D::FileClass: From<BTreeType>,
{
    /// Create a new `SparseTensor` with the given schema
    pub async fn create(dir: &D, schema: Schema, txn_id: TxnId) -> TCResult<Self> {
        SparseTable::create(dir, schema, txn_id)
            .map_ok(Self::from)
            .await
    }
}

impl<FD, FS, D, T, A> TensorAccess for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
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

impl<FD, FS, D, T, A> TensorInstance for SparseTensor<FD, FS, D, T, A> {
    type Dense = DenseTensor<FD, FS, D, T, BlockListSparse<FD, FS, D, T, A>>;
    type Sparse = Self;

    fn into_dense(self) -> Self::Dense {
        BlockListSparse::from(self.into_inner()).into()
    }

    fn into_sparse(self) -> Self::Sparse {
        self
    }
}

#[async_trait]
impl<FD, FS, D, T, L, R> TensorBoolean<SparseTensor<FD, FS, D, T, R>>
    for SparseTensor<FD, FS, D, T, L>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    L: SparseAccess<FD, FS, D, T>,
    R: SparseAccess<FD, FS, D, T>,
{
    type Combine = SparseTensor<FD, FS, D, T, SparseCombinator<FD, FS, D, T, L, R>>;

    fn and(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        self.combine(other, Number::and, NumberType::Bool)
    }

    fn or(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        self.combine(other, Number::or, NumberType::Bool)
    }

    fn xor(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        self.combine(other, Number::xor, NumberType::Bool)
    }
}

#[async_trait]
impl<FD, FS, D, T, A> TensorBoolean<Tensor<FD, FS, D, T>> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Combine = Tensor<FD, FS, D, T>;

    fn and(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Dense(other) => self.and(other.into_sparse()).map(Tensor::from),
            Tensor::Sparse(other) => self.and(other).map(Tensor::from),
        }
    }

    fn or(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Dense(other) => self.into_dense().or(other).map(Tensor::from),
            Tensor::Sparse(other) => self.or(other).map(Tensor::from),
        }
    }

    fn xor(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Dense(other) => self.into_dense().xor(other).map(Tensor::from),
            Tensor::Sparse(other) => self.xor(other).map(Tensor::from),
        }
    }
}

impl<FD, FS, D, T, L, R> TensorCompare<SparseTensor<FD, FS, D, T, R>>
    for SparseTensor<FD, FS, D, T, L>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    L: SparseAccess<FD, FS, D, T>,
    R: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Compare = SparseTensor<FD, FS, D, T, SparseCombinator<FD, FS, D, T, L, R>>;
    type Dense = Condensed<FD, FS, D, T, L, R>;

    fn eq(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Dense> {
        fn eq(l: Number, r: Number) -> Number {
            (l == r).into()
        }

        self.condense(other, eq)
    }

    fn gt(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Compare> {
        fn gt(l: Number, r: Number) -> Number {
            (l > r).into()
        }

        self.combine(other, gt, NumberType::Bool)
    }

    fn gte(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Dense> {
        fn gte(l: Number, r: Number) -> Number {
            (l >= r).into()
        }

        self.condense(other, gte)
    }

    fn lt(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Compare> {
        fn lt(l: Number, r: Number) -> Number {
            (l < r).into()
        }

        self.combine(other, lt, NumberType::Bool)
    }

    fn lte(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Dense> {
        fn lte(l: Number, r: Number) -> Number {
            (l <= r).into()
        }

        self.condense(other, lte)
    }

    fn ne(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Compare> {
        fn ne(l: Number, r: Number) -> Number {
            (l != r).into()
        }

        self.combine(other, ne, NumberType::Bool)
    }
}

impl<FD, FS, D, T, A> TensorCompare<Tensor<FD, FS, D, T>> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Compare = Tensor<FD, FS, D, T>;
    type Dense = Tensor<FD, FS, D, T>;

    fn eq(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Dense> {
        match other {
            Tensor::Dense(other) => self.eq(other.into_sparse()).map(Tensor::from),
            Tensor::Sparse(other) => self.eq(other).map(Tensor::from),
        }
    }

    fn gt(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Compare> {
        match other {
            Tensor::Dense(other) => self.into_dense().gt(other).map(Tensor::from),
            Tensor::Sparse(other) => self.gt(other).map(Tensor::from),
        }
    }

    fn gte(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Dense> {
        match other {
            Tensor::Dense(other) => self.into_dense().gte(other).map(Tensor::from),
            Tensor::Sparse(other) => self.gte(other).map(Tensor::from),
        }
    }

    fn lt(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Compare> {
        match other {
            Tensor::Dense(other) => self.into_dense().gt(other).map(Tensor::from),
            Tensor::Sparse(other) => self.gt(other).map(Tensor::from),
        }
    }

    fn lte(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Dense> {
        match other {
            Tensor::Dense(other) => self.into_dense().lte(other).map(Tensor::from),
            Tensor::Sparse(other) => self.lte(other).map(Tensor::from),
        }
    }

    fn ne(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Compare> {
        match other {
            Tensor::Dense(other) => self.ne(other.into_sparse()).map(Tensor::from),
            Tensor::Sparse(other) => self.ne(other).map(Tensor::from),
        }
    }
}

#[async_trait]
impl<FD, FS, D, T, L, R> TensorDualIO<D, SparseTensor<FD, FS, D, T, R>>
    for SparseTensor<FD, FS, D, T, L>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    L: SparseAccess<FD, FS, D, T>,
    R: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Txn = T;

    async fn mask(self, txn: T, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<()> {
        if self.shape() != other.shape() {
            return Err(TCError::unsupported(format!(
                "cannot use a Tensor with shape {} as a mask for a Tensor with shape {}",
                other.shape(),
                self.shape(),
            )));
        }

        let zero = self.dtype().zero();
        let txn_id = *txn.id();

        let filled = other.accessor.filled(txn).await?;

        filled
            .map_ok(|(coord, _)| self.write_value_at(txn_id, coord, zero.clone()))
            .try_buffer_unordered(num_cpus::get())
            .try_fold((), |_, _| future::ready(Ok(())))
            .await
    }

    async fn write(
        self,
        txn: T,
        bounds: Bounds,
        other: SparseTensor<FD, FS, D, T, R>,
    ) -> TCResult<()> {
        let slice = self.slice(bounds)?;
        if slice.shape() != other.shape() {
            return Err(TCError::unsupported(format!(
                "cannot write tensor of shape {} to slice of shape {}",
                other.shape(),
                slice.shape()
            )));
        }

        let txn_id = *txn.id();
        let filled = other.accessor.filled(txn).await?;
        filled
            .map_ok(|(coord, value)| slice.write_value_at(txn_id, coord, value))
            .try_buffer_unordered(num_cpus::get())
            .try_fold((), |_, _| future::ready(Ok(())))
            .await
    }
}

#[async_trait]
impl<FD, FS, D, T, A> TensorDualIO<D, Tensor<FD, FS, D, T>> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Txn = T;

    async fn mask(self, txn: Self::Txn, other: Tensor<FD, FS, D, T>) -> TCResult<()> {
        let other = if self.shape() == other.shape() {
            other
        } else {
            other.broadcast(self.shape().clone())?
        };

        match other {
            Tensor::Dense(other) => self.mask(txn, other.into_sparse()).await,
            Tensor::Sparse(other) => self.mask(txn, other).await,
        }
    }

    async fn write(
        self,
        txn: Self::Txn,
        bounds: Bounds,
        other: Tensor<FD, FS, D, T>,
    ) -> TCResult<()> {
        let shape = bounds.to_shape(self.shape())?;
        let other = if other.shape() == &shape {
            other
        } else {
            other.broadcast(shape)?
        };

        match other {
            Tensor::Dense(other) => self.write(txn, bounds, other.into_sparse()).await,
            Tensor::Sparse(other) => self.write(txn, bounds, other).await,
        }
    }
}

#[async_trait]
impl<FD, FS, D, T, A> TensorIO<D> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
{
    type Txn = T;

    async fn read_value(self, txn: Self::Txn, coord: Coord) -> TCResult<Number> {
        self.accessor
            .read_value_at(txn, coord)
            .map_ok(|(_, value)| value)
            .await
    }

    async fn write_value(&self, txn_id: TxnId, mut bounds: Bounds, value: Number) -> TCResult<()> {
        if self.shape().is_empty() {
            return self.accessor.write_value(txn_id, vec![], value).await;
        }

        bounds.normalize(self.shape());
        debug!("SparseTensor::write_value {} to bounds, {}", value, bounds);
        stream::iter(bounds.affected())
            .map(|coord| self.accessor.write_value(txn_id, coord, value))
            .buffer_unordered(num_cpus::get())
            .try_fold((), |_, _| future::ready(Ok(())))
            .await
    }

    async fn write_value_at(&self, txn_id: TxnId, coord: Coord, value: Number) -> TCResult<()> {
        self.accessor.write_value(txn_id, coord, value).await
    }
}

impl<FD, FS, D, T, L, R> TensorMath<D, SparseTensor<FD, FS, D, T, R>>
    for SparseTensor<FD, FS, D, T, L>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    L: SparseAccess<FD, FS, D, T>,
    R: SparseAccess<FD, FS, D, T>,
{
    type Combine = SparseTensor<FD, FS, D, T, SparseCombinator<FD, FS, D, T, L, R>>;

    fn add(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        let dtype = Ord::max(self.dtype(), other.dtype());
        self.combine(other, Number::add, dtype)
    }

    fn div(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        // TODO: implement left_combine
        let dtype = Ord::max(self.dtype(), other.dtype());
        self.combine(other, Number::div, dtype)
    }

    fn mul(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        // TODO: implement left_combine
        let dtype = Ord::max(self.dtype(), other.dtype());
        self.combine(other, Number::mul, dtype)
    }

    fn sub(self, other: SparseTensor<FD, FS, D, T, R>) -> TCResult<Self::Combine> {
        let dtype = Ord::max(self.dtype(), other.dtype());
        self.combine(other, Number::sub, dtype)
    }
}

impl<FD, FS, D, T, A> TensorMath<D, Tensor<FD, FS, D, T>> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Combine = Tensor<FD, FS, D, T>;

    fn add(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Sparse(sparse) => self.add(sparse).map(Tensor::from),
            Tensor::Dense(dense) => self.into_dense().add(dense).map(Tensor::from),
        }
    }

    fn div(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Sparse(sparse) => self.div(sparse).map(Tensor::from),
            Tensor::Dense(dense) => self.div(dense.into_sparse()).map(Tensor::from),
        }
    }

    fn mul(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Sparse(sparse) => self.mul(sparse).map(Tensor::from),
            Tensor::Dense(dense) => self.mul(dense.into_sparse()).map(Tensor::from),
        }
    }

    fn sub(self, other: Tensor<FD, FS, D, T>) -> TCResult<Self::Combine> {
        match other {
            Tensor::Sparse(sparse) => self.sub(sparse).map(Tensor::from),
            Tensor::Dense(dense) => self.into_dense().sub(dense).map(Tensor::from),
        }
    }
}

impl<FD, FS, D, T, A> TensorReduce<D> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
    Self: TensorInstance,
    <Self as TensorInstance>::Dense: TensorReduce<D, Txn = T> + Send + Sync,
{
    type Txn = T;
    type Reduce = SparseTensor<FD, FS, D, T, SparseReduce<FD, FS, D, T>>;

    fn product(self, axis: usize) -> TCResult<Self::Reduce> {
        let accessor = SparseReduce::new(
            self.accessor.accessor(),
            axis,
            SparseTensor::<FD, FS, D, T, SparseAccessor<FD, FS, D, T>>::product_all,
        )?;

        Ok(SparseTensor::from(accessor))
    }

    fn product_all(&self, txn: T) -> TCBoxTryFuture<Number> {
        Box::pin(async move { self.clone().into_dense().product_all(txn).await })
    }

    fn sum(self, axis: usize) -> TCResult<Self::Reduce> {
        let accessor = SparseReduce::new(
            self.accessor.accessor(),
            axis,
            SparseTensor::<FD, FS, D, T, SparseAccessor<FD, FS, D, T>>::sum_all,
        )?;

        Ok(SparseTensor::from(accessor))
    }

    fn sum_all(&self, txn: T) -> TCBoxTryFuture<Number> {
        Box::pin(async move {
            let mut sum = self.dtype().zero();
            let mut filled = self.accessor.clone().filled(txn).await?;
            let mut buffer = Vec::with_capacity(PER_BLOCK);
            while let Some((_coord, value)) = filled.try_next().await? {
                buffer.push(value);

                if buffer.len() == PER_BLOCK {
                    sum += Array::from(buffer.to_vec()).sum();
                    buffer.clear()
                }
            }

            if !buffer.is_empty() {
                sum += Array::from(buffer).sum();
            }

            Ok(sum)
        })
    }
}

impl<FD, FS, D, T, A> TensorTransform for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Broadcast = SparseTensor<FD, FS, D, T, SparseBroadcast<FD, FS, D, T, A>>;
    type Cast = SparseTensor<FD, FS, D, T, SparseCast<FD, FS, D, T, A>>;
    type Expand = SparseTensor<FD, FS, D, T, SparseExpand<FD, FS, D, T, A>>;
    type Slice = SparseTensor<FD, FS, D, T, A::Slice>;
    type Transpose = SparseTensor<FD, FS, D, T, A::Transpose>;

    fn broadcast(self, shape: Shape) -> TCResult<Self::Broadcast> {
        let accessor = SparseBroadcast::new(self.accessor, shape)?;
        Ok(accessor.into())
    }

    fn cast_into(self, dtype: NumberType) -> TCResult<Self::Cast> {
        let accessor = SparseCast::new(self.accessor, dtype);
        Ok(accessor.into())
    }

    fn expand_dims(self, axis: usize) -> TCResult<Self::Expand> {
        let accessor = SparseExpand::new(self.accessor, axis)?;
        Ok(accessor.into())
    }

    fn slice(self, bounds: Bounds) -> TCResult<Self::Slice> {
        let accessor = self.accessor.slice(bounds)?;
        Ok(accessor.into())
    }

    fn transpose(self, permutation: Option<Vec<usize>>) -> TCResult<Self::Transpose> {
        let accessor = self.accessor.transpose(permutation)?;
        Ok(accessor.into())
    }
}

#[async_trait]
impl<FD, FS, D, T, A> TensorUnary<D> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
{
    type Txn = T;
    type Unary = SparseTensor<FD, FS, D, T, SparseUnary<FD, FS, D, T>>;

    fn abs(&self) -> TCResult<Self::Unary> {
        let source = self.accessor.clone().accessor();
        let transform = <Number as NumberInstance>::abs;

        let accessor = SparseUnary::new(source, transform, self.dtype());
        Ok(SparseTensor::from(accessor))
    }

    async fn all(self, txn: Self::Txn) -> TCResult<bool> {
        let affected = stream::iter(Bounds::all(self.shape()).affected());
        let filled = self.accessor.filled(txn).await?;

        let mut coords = filled
            .map_ok(|(coord, _)| coord)
            .zip(affected)
            .map(|(r, expected)| r.map(|actual| (actual, expected)));

        while let Some((actual, expected)) = coords.try_next().await? {
            if actual != expected {
                return Ok(false);
            }
        }

        Ok(true)
    }

    async fn any(self, txn: Self::Txn) -> TCResult<bool> {
        let mut filled = self.accessor.filled(txn).await?;
        Ok(filled.next().await.is_some())
    }

    fn not(&self) -> TCResult<Self::Unary> {
        Err(TCError::unsupported(ERR_NOT_SPARSE))
    }
}

#[async_trait]
impl<FD, FS, D, T, A> CopyFrom<D, SparseTensor<FD, FS, D, T, A>>
    for SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<BTreeType> + From<TensorType>,
{
    async fn copy_from(
        instance: SparseTensor<FD, FS, D, T, A>,
        store: Self::Store,
        txn: &Self::Txn,
    ) -> TCResult<Self> {
        SparseTable::copy_from(instance, store, txn)
            .map_ok(Self::from)
            .await
    }
}

#[async_trait]
impl<'en, FD, FS, D, T, A> Hash<'en, D> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
{
    type Item = SparseRow;
    type Txn = T;

    async fn hashable(&'en self, txn: &'en Self::Txn) -> TCResult<TCBoxTryStream<'en, SparseRow>> {
        self.accessor.clone().filled(txn.clone()).await
    }
}

#[async_trait]
impl<FD, FS, D, T> Persist<D> for SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    D::FileClass: From<BTreeType> + From<TensorType>,
{
    type Schema = Schema;
    type Store = D;
    type Txn = T;

    fn schema(&self) -> &Self::Schema {
        self.accessor.schema()
    }

    async fn load(txn: &Self::Txn, schema: Self::Schema, store: Self::Store) -> TCResult<Self> {
        SparseTable::load(txn, schema, store)
            .map_ok(Self::from)
            .await
    }
}

#[async_trait]
impl<FD, FS, D, T> Restore<D> for SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    D::FileClass: From<BTreeType> + From<TensorType>,
{
    async fn restore(&self, backup: &Self, txn_id: TxnId) -> TCResult<()> {
        self.accessor.restore(&backup.accessor, txn_id).await
    }
}

#[async_trait]
impl<FD, FS, D, T> Transact for SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    Self: Send + Sync,
    SparseTable<FD, FS, D, T>: Transact + Send + Sync,
{
    async fn commit(&self, txn_id: &TxnId) {
        self.accessor.commit(txn_id).await
    }

    async fn finalize(&self, txn_id: &TxnId) {
        self.accessor.finalize(txn_id).await
    }
}

impl<FD, FS, D, T, A> From<A> for SparseTensor<FD, FS, D, T, A> {
    fn from(accessor: A) -> Self {
        Self {
            accessor,
            phantom: Phantom::default(),
        }
    }
}

impl<FD, FS, D, T, A> fmt::Display for SparseTensor<FD, FS, D, T, A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a sparse Tensor")
    }
}

#[async_trait]
impl<'en, FD, FS, D, T, A> IntoView<'en, D> for SparseTensor<FD, FS, D, T, A>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node>,
    D: Dir,
    T: Transaction<D>,
    A: SparseAccess<FD, FS, D, T>,
    D::FileClass: From<TensorType>,
{
    type Txn = T;
    type View = SparseTensorView<'en>;

    async fn into_view(self, txn: Self::Txn) -> TCResult<Self::View> {
        let shape = self.shape().clone();
        let dtype = self.dtype();

        Ok(SparseTensorView {
            schema: Schema { shape, dtype },
            filled: self.accessor.filled(txn).await?,
        })
    }
}

#[async_trait]
impl<FD, FS, D, T> de::FromStream for SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    D::FileClass: From<BTreeType> + From<TensorType>,
{
    type Context = T;

    async fn from_stream<De: de::Decoder>(txn: T, decoder: &mut De) -> Result<Self, De::Error> {
        decoder.decode_seq(SparseTensorVisitor::new(txn)).await
    }
}

struct SparseTensorVisitor<FD, FS, D, T> {
    txn: T,
    dense: PhantomData<FD>,
    sparse: PhantomData<FS>,
    dir: PhantomData<D>,
}

impl<FD, FS, D, T> SparseTensorVisitor<FD, FS, D, T> {
    fn new(txn: T) -> Self {
        Self {
            txn,
            dense: PhantomData,
            sparse: PhantomData,
            dir: PhantomData,
        }
    }
}

#[async_trait]
impl<FD, FS, D, T> de::Visitor for SparseTensorVisitor<FD, FS, D, T>
where
    FD: File<Array> + TryFrom<D::File, Error = TCError>,
    FS: File<Node> + TryFrom<D::File, Error = TCError>,
    D: Dir,
    T: Transaction<D>,
    D::FileClass: From<BTreeType> + From<TensorType>,
{
    type Value = SparseTensor<FD, FS, D, T, SparseTable<FD, FS, D, T>>;

    fn expecting() -> &'static str {
        "a SparseTensor"
    }

    async fn visit_seq<A: de::SeqAccess>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let schema = seq.next_element(()).await?;
        let schema = schema.ok_or_else(|| de::Error::invalid_length(0, "tensor schema"))?;

        let txn_id = *self.txn.id();
        let table = SparseTable::create(self.txn.context(), schema, txn_id)
            .map_err(de::Error::custom)
            .await?;

        if let Some(table) = seq
            .next_element::<SparseTable<FD, FS, D, T>>((table.clone(), txn_id))
            .await?
        {
            Ok(SparseTensor::from(table))
        } else {
            Ok(SparseTensor::from(table))
        }
    }
}

pub struct SparseTensorView<'en> {
    schema: Schema,
    filled: SparseStream<'en>,
}

impl<'en> en::IntoStream<'en> for SparseTensorView<'en> {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let filled = en::SeqStream::from(self.filled);
        (self.schema, filled).into_stream(encoder)
    }
}
