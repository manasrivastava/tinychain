use std::sync::Arc;

use async_trait::async_trait;

use crate::transaction::{Txn, TxnId};
use crate::value::class::NumberType;
use crate::value::{Number, TCBoxTryFuture, TCResult};

mod bounds;
mod dense;
mod sparse;
mod transform;

pub type Array = dense::array::Array;

use dense::DenseTensor;
use sparse::SparseTensor;

trait TensorView: Send + Sync {
    fn dtype(&self) -> NumberType;

    fn ndim(&self) -> usize;

    fn shape(&'_ self) -> &'_ bounds::Shape;

    fn size(&self) -> u64;
}

#[async_trait]
trait TensorBoolean: Sized + TensorView {
    async fn all(&self, txn: Arc<Txn>) -> TCResult<bool>;

    fn any(&self, txn: Arc<Txn>) -> TCBoxTryFuture<bool>;

    fn and(&self, other: &Self) -> TCResult<Self>;

    fn not(&self) -> TCResult<Self>;

    fn or(&self, other: &Self) -> TCResult<Self>;

    fn xor(&self, other: &Self) -> TCResult<Self>;
}

#[async_trait]
trait TensorCompare: Sized + TensorView {
    async fn eq(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor>;

    fn gt(&self, other: &Self) -> TCResult<Self>;

    async fn gte(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor>;

    fn lt(&self, other: &Self) -> TCResult<Self>;

    async fn lte(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor>;

    fn ne(&self, other: &Self) -> TCResult<Self>;
}

trait TensorIO: Sized + TensorView {
    fn read_value<'a>(&'a self, txn: &'a Arc<Txn>, coord: &'a [u64]) -> TCBoxTryFuture<'a, Number>;

    fn write<'a>(
        &'a self,
        txn: Arc<Txn>,
        bounds: bounds::Bounds,
        value: Self,
    ) -> TCBoxTryFuture<'a, ()>;

    fn write_value(
        &self,
        txn_id: TxnId,
        bounds: bounds::Bounds,
        value: Number,
    ) -> TCBoxTryFuture<()>;

    fn write_value_at<'a>(
        &'a self,
        txn_id: TxnId,
        coord: Vec<u64>,
        value: Number,
    ) -> TCBoxTryFuture<'a, ()>;
}

trait TensorMath: Sized + TensorView {
    fn abs(&self) -> TCResult<Self>;

    fn add(&self, other: &Self) -> TCResult<Self>;

    fn multiply(&self, other: &Self) -> TCResult<Self>;
}

trait TensorReduce: Sized + TensorView {
    fn product(&self, txn: Arc<Txn>, axis: usize) -> TCResult<Self>;

    fn product_all(&self, txn: Arc<Txn>) -> TCBoxTryFuture<Number>;

    fn sum(&self, txn: Arc<Txn>, axis: usize) -> TCResult<Self>;

    fn sum_all(&self, txn: Arc<Txn>) -> TCBoxTryFuture<Number>;
}

trait TensorTransform: Sized + TensorView {
    fn as_type(&self, dtype: NumberType) -> TCResult<Self>;

    fn broadcast(&self, shape: bounds::Shape) -> TCResult<Self>;

    fn expand_dims(&self, axis: usize) -> TCResult<Self>;

    fn slice(&self, bounds: bounds::Bounds) -> TCResult<Self>;

    fn reshape(&self, shape: bounds::Shape) -> TCResult<Self>;

    fn transpose(&self, permutation: Option<Vec<usize>>) -> TCResult<Self>;
}

enum Tensor {
    Dense(DenseTensor),
    Sparse(SparseTensor),
}

impl TensorView for Tensor {
    fn dtype(&self) -> NumberType {
        match self {
            Self::Dense(dense) => dense.dtype(),
            Self::Sparse(sparse) => sparse.dtype(),
        }
    }

    fn ndim(&self) -> usize {
        match self {
            Self::Dense(dense) => dense.ndim(),
            Self::Sparse(sparse) => sparse.ndim(),
        }
    }

    fn shape(&'_ self) -> &'_ bounds::Shape {
        match self {
            Self::Dense(dense) => dense.shape(),
            Self::Sparse(sparse) => sparse.shape(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Self::Dense(dense) => dense.size(),
            Self::Sparse(sparse) => sparse.size(),
        }
    }
}

#[async_trait]
impl TensorBoolean for Tensor {
    async fn all(&self, txn: Arc<Txn>) -> TCResult<bool> {
        match self {
            Self::Dense(dense) => dense.all(txn).await,
            Self::Sparse(sparse) => sparse.all(txn).await,
        }
    }

    fn any(&'_ self, txn: Arc<Txn>) -> TCBoxTryFuture<'_, bool> {
        match self {
            Self::Dense(dense) => dense.any(txn),
            Self::Sparse(sparse) => sparse.any(txn),
        }
    }

    fn and(&self, other: &Self) -> TCResult<Self> {
        use Tensor::*;
        match (self, other) {
            (Dense(left), Dense(right)) => left.and(right).map(Self::from),
            (Sparse(left), Sparse(right)) => left.and(right).map(Self::from),
            (Sparse(left), Dense(right)) => left
                .and(&SparseTensor::from_dense(right.clone()))
                .map(Self::from),

            _ => other.and(self),
        }
    }

    fn not(&self) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.not().map(Self::from),
            Self::Sparse(sparse) => sparse.not().map(Self::from),
        }
    }

    fn or(&self, other: &Self) -> TCResult<Self> {
        use Tensor::*;
        match (self, other) {
            (Dense(left), Dense(right)) => left.or(right).map(Self::from),
            (Sparse(left), Sparse(right)) => left.or(right).map(Self::from),
            (Dense(left), Sparse(right)) => left
                .or(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),

            _ => other.or(self),
        }
    }

    fn xor(&self, other: &Self) -> TCResult<Self> {
        use Tensor::*;
        match (self, other) {
            (Dense(left), Dense(right)) => left.xor(right).map(Self::from),
            (Sparse(left), _) => Dense(DenseTensor::from_sparse(left.clone())).xor(other),
            (left, right) => right.xor(left),
        }
    }
}

#[async_trait]
impl TensorCompare for Tensor {
    async fn eq(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.eq(right, txn).await,
            (Self::Sparse(left), Self::Sparse(right)) => left.eq(right, txn).await,
            (Self::Dense(left), Self::Sparse(right)) => {
                left.eq(&DenseTensor::from_sparse(right.clone()), txn).await
            }
            (Self::Sparse(left), Self::Dense(right)) => {
                DenseTensor::from_sparse(left.clone()).eq(right, txn).await
            }
        }
    }

    fn gt(&self, other: &Self) -> TCResult<Self> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.gt(right).map(Self::from),
            (Self::Sparse(left), Self::Sparse(right)) => left.gt(right).map(Self::from),
            (Self::Dense(left), Self::Sparse(right)) => left
                .gt(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),
            (Self::Sparse(left), Self::Dense(right)) => left
                .gt(&SparseTensor::from_dense(right.clone()))
                .map(Self::from),
        }
    }

    async fn gte(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.gte(right, txn).await,
            (Self::Sparse(left), Self::Sparse(right)) => left.gte(right, txn).await,
            (Self::Dense(left), Self::Sparse(right)) => {
                left.gte(&DenseTensor::from_sparse(right.clone()), txn)
                    .await
            }
            (Self::Sparse(left), Self::Dense(right)) => {
                DenseTensor::from_sparse(left.clone()).gte(right, txn).await
            }
        }
    }

    fn lt(&self, other: &Self) -> TCResult<Self> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.lt(right).map(Self::from),
            (Self::Sparse(left), Self::Sparse(right)) => left.lt(right).map(Self::from),
            (Self::Dense(left), Self::Sparse(right)) => left
                .lt(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),
            (Self::Sparse(left), Self::Dense(right)) => left
                .lt(&SparseTensor::from_dense(right.clone()))
                .map(Self::from),
        }
    }

    async fn lte(&self, other: &Self, txn: Arc<Txn>) -> TCResult<DenseTensor> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.lte(right, txn).await,
            (Self::Sparse(left), Self::Sparse(right)) => left.lte(right, txn).await,
            (Self::Dense(left), Self::Sparse(right)) => {
                left.lte(&DenseTensor::from_sparse(right.clone()), txn)
                    .await
            }
            (Self::Sparse(left), Self::Dense(right)) => {
                DenseTensor::from_sparse(left.clone()).lte(right, txn).await
            }
        }
    }

    fn ne(&self, other: &Self) -> TCResult<Self> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.ne(right).map(Self::from),
            (Self::Sparse(left), Self::Sparse(right)) => left.ne(right).map(Self::from),
            (Self::Dense(left), Self::Sparse(right)) => left
                .ne(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),
            (Self::Sparse(left), Self::Dense(right)) => DenseTensor::from_sparse(left.clone())
                .ne(right)
                .map(Self::from),
        }
    }
}

impl TensorIO for Tensor {
    fn read_value<'a>(&'a self, txn: &'a Arc<Txn>, coord: &'a [u64]) -> TCBoxTryFuture<'a, Number> {
        match self {
            Self::Dense(dense) => dense.read_value(txn, coord),
            Self::Sparse(sparse) => sparse.read_value(txn, coord),
        }
    }

    fn write<'a>(
        &'a self,
        txn: Arc<Txn>,
        bounds: bounds::Bounds,
        value: Self,
    ) -> TCBoxTryFuture<'a, ()> {
        match self {
            Self::Dense(this) => match value {
                Self::Dense(that) => this.write(txn, bounds, that),
                Self::Sparse(that) => this.write(txn, bounds, DenseTensor::from_sparse(that)),
            },
            Self::Sparse(this) => match value {
                Self::Dense(that) => this.write(txn, bounds, SparseTensor::from_dense(that)),
                Self::Sparse(that) => this.write(txn, bounds, that),
            },
        }
    }

    fn write_value(
        &self,
        txn_id: TxnId,
        bounds: bounds::Bounds,
        value: Number,
    ) -> TCBoxTryFuture<()> {
        match self {
            Self::Dense(dense) => dense.write_value(txn_id, bounds, value),
            Self::Sparse(sparse) => sparse.write_value(txn_id, bounds, value),
        }
    }

    fn write_value_at<'a>(
        &'a self,
        txn_id: TxnId,
        coord: Vec<u64>,
        value: Number,
    ) -> TCBoxTryFuture<'a, ()> {
        match self {
            Self::Dense(dense) => dense.write_value_at(txn_id, coord, value),
            Self::Sparse(sparse) => sparse.write_value_at(txn_id, coord, value),
        }
    }
}

impl TensorMath for Tensor {
    fn abs(&self) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.abs().map(Self::from),
            Self::Sparse(sparse) => sparse.abs().map(Self::from),
        }
    }

    fn add(&self, other: &Self) -> TCResult<Self> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.add(right).map(Self::from),
            (Self::Sparse(left), Self::Sparse(right)) => left.add(right).map(Self::from),
            (Self::Dense(left), Self::Sparse(right)) => left
                .add(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),
            (Self::Sparse(left), Self::Dense(right)) => DenseTensor::from_sparse(left.clone())
                .add(right)
                .map(Self::from),
        }
    }

    fn multiply(&self, other: &Self) -> TCResult<Self> {
        match (self, other) {
            (Self::Dense(left), Self::Dense(right)) => left.multiply(right).map(Self::from),
            (Self::Sparse(left), Self::Sparse(right)) => left.multiply(right).map(Self::from),
            (Self::Dense(left), Self::Sparse(right)) => left
                .multiply(&DenseTensor::from_sparse(right.clone()))
                .map(Self::from),
            (Self::Sparse(left), Self::Dense(right)) => DenseTensor::from_sparse(left.clone())
                .multiply(right)
                .map(Self::from),
        }
    }
}

impl TensorReduce for Tensor {
    fn product(&self, txn: Arc<Txn>, axis: usize) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.product(txn, axis).map(Self::from),
            Self::Sparse(sparse) => sparse.product(txn, axis).map(Self::from),
        }
    }

    fn product_all(&self, txn: Arc<Txn>) -> TCBoxTryFuture<Number> {
        match self {
            Self::Dense(dense) => dense.product_all(txn),
            Self::Sparse(sparse) => sparse.product_all(txn),
        }
    }

    fn sum(&self, txn: Arc<Txn>, axis: usize) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.product(txn, axis).map(Self::from),
            Self::Sparse(sparse) => sparse.product(txn, axis).map(Self::from),
        }
    }

    fn sum_all(&self, txn: Arc<Txn>) -> TCBoxTryFuture<Number> {
        match self {
            Self::Dense(dense) => dense.product_all(txn),
            Self::Sparse(sparse) => sparse.product_all(txn),
        }
    }
}

impl TensorTransform for Tensor {
    fn as_type(&self, dtype: NumberType) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.as_type(dtype).map(Self::from),
            Self::Sparse(sparse) => sparse.as_type(dtype).map(Self::from),
        }
    }

    fn broadcast(&self, shape: bounds::Shape) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.broadcast(shape).map(Self::from),
            Self::Sparse(sparse) => sparse.broadcast(shape).map(Self::from),
        }
    }

    fn expand_dims(&self, axis: usize) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.expand_dims(axis).map(Self::from),
            Self::Sparse(sparse) => sparse.expand_dims(axis).map(Self::from),
        }
    }

    fn slice(&self, bounds: bounds::Bounds) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.slice(bounds).map(Self::from),
            Self::Sparse(sparse) => sparse.slice(bounds).map(Self::from),
        }
    }

    fn reshape(&self, shape: bounds::Shape) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.reshape(shape).map(Self::from),
            Self::Sparse(sparse) => sparse.reshape(shape).map(Self::from),
        }
    }

    fn transpose(&self, permutation: Option<Vec<usize>>) -> TCResult<Self> {
        match self {
            Self::Dense(dense) => dense.transpose(permutation).map(Self::from),
            Self::Sparse(sparse) => sparse.transpose(permutation).map(Self::from),
        }
    }
}

impl From<DenseTensor> for Tensor {
    fn from(dense: DenseTensor) -> Tensor {
        Self::Dense(dense)
    }
}

impl From<SparseTensor> for Tensor {
    fn from(sparse: SparseTensor) -> Tensor {
        Self::Sparse(sparse)
    }
}

fn broadcast<L: Clone + TensorTransform, R: Clone + TensorTransform>(
    left: &L,
    right: &R,
) -> TCResult<(L, R)> {
    if left.shape() == right.shape() {
        Ok((left.clone(), right.clone()))
    } else {
        let shape: bounds::Shape = left
            .shape()
            .to_vec()
            .iter()
            .zip(right.shape().to_vec().iter())
            .map(|(l, r)| Ord::max(l, r))
            .cloned()
            .collect::<Vec<u64>>()
            .into();
        Ok((left.broadcast(shape.clone())?, right.broadcast(shape)?))
    }
}
