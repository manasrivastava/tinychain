use std::iter::FromIterator;

use log::debug;

use tc_error::*;
use tcgeneric::{Id, Instance, Map, PathSegment, TCPath, Tuple};

use crate::object::InstanceExt;
use crate::route::{DeleteHandler, GetHandler, Handler, PostHandler, PutHandler, Route};
use crate::scalar::*;
use crate::state::State;
use crate::txn::Txn;

struct GetMethod<'a, T: Instance> {
    subject: &'a InstanceExt<T>,
    method: GetOp,
    path: &'a [PathSegment],
}

impl<'a, T: Instance + Route + 'a> GetMethod<'a, T> {
    async fn call(self, txn: &Txn, key: Value) -> TCResult<State> {
        let (key_name, op_def) = self.method;

        let mut context = Map::new();
        context.insert(key_name, key.into());

        call_method(txn, self.subject, self.path, context, op_def).await
    }
}

impl<'a, T: Instance + Route + 'a> Handler<'a> for GetMethod<'a, T> {
    fn get<'b>(self: Box<Self>) -> Option<GetHandler<'a, 'b>>
    where
        'b: 'a,
    {
        Some(Box::new(move |txn, key| Box::pin(self.call(txn, key))))
    }
}

struct PutMethod<'a, T: Instance> {
    subject: &'a InstanceExt<T>,
    method: PutOp,
    path: &'a [PathSegment],
}

impl<'a, T: Instance + Route + 'a> PutMethod<'a, T> {
    async fn call(self, txn: &Txn, key: Value, value: State) -> TCResult<()> {
        let (key_name, value_name, op_def) = self.method;

        let mut context = Map::new();
        context.insert(key_name, key.into());
        context.insert(value_name, value);

        call_method(txn, self.subject, self.path, context, op_def).await?;
        Ok(())
    }
}

impl<'a, T: Instance + Route + 'a> Handler<'a> for PutMethod<'a, T> {
    fn put<'b>(self: Box<Self>) -> Option<PutHandler<'a, 'b>>
    where
        'b: 'a,
    {
        Some(Box::new(move |txn, key, value| {
            Box::pin(self.call(txn, key, value))
        }))
    }
}

struct PostMethod<'a, T: Instance> {
    subject: &'a InstanceExt<T>,
    method: PostOp,
    path: &'a [PathSegment],
}

impl<'a, T: Instance + Route + 'a> PostMethod<'a, T> {
    async fn call(self, txn: &Txn, params: Map<State>) -> TCResult<State> {
        call_method(txn, self.subject, self.path, params, self.method).await
    }
}

impl<'a, T: Instance + Route + 'a> Handler<'a> for PostMethod<'a, T> {
    fn post<'b>(self: Box<Self>) -> Option<PostHandler<'a, 'b>>
    where
        'b: 'a,
    {
        Some(Box::new(move |txn, params| {
            Box::pin(self.call(txn, params))
        }))
    }
}

struct DeleteMethod<'a, T: Instance> {
    subject: &'a InstanceExt<T>,
    method: DeleteOp,
    path: &'a [PathSegment],
}

impl<'a, T: Instance + Route + 'a> DeleteMethod<'a, T> {
    async fn call(self, txn: &Txn, key: Value) -> TCResult<()> {
        let (key_name, op_def) = self.method;

        let mut context = Map::new();
        context.insert(key_name, key.into());

        call_method(txn, self.subject, self.path, context, op_def).await?;
        Ok(())
    }
}

impl<'a, T: Instance + Route + 'a> Handler<'a> for DeleteMethod<'a, T> {
    fn delete<'b>(self: Box<Self>) -> Option<DeleteHandler<'a, 'b>>
    where
        'b: 'a,
    {
        Some(Box::new(move |txn, key| Box::pin(self.call(txn, key))))
    }
}

impl<T: Instance + Route> Route for InstanceExt<T> {
    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<Box<dyn Handler<'a> + 'a>> {
        debug!("InstanceExt::route {}", TCPath::from(path));

        if path.is_empty() {
            self.parent().route(path)
        } else if let Some(member) = self.proto().get(&path[0]) {
            match member {
                Scalar::Op(OpDef::Get(get_op)) => Some(Box::new(GetMethod {
                    subject: self,
                    method: get_op.clone(),
                    path: &path[1..],
                })),
                Scalar::Op(OpDef::Put(put_op)) => Some(Box::new(PutMethod {
                    subject: self,
                    method: put_op.clone(),
                    path: &path[1..],
                })),
                Scalar::Op(OpDef::Post(post_op)) => Some(Box::new(PostMethod {
                    subject: self,
                    method: post_op.clone(),
                    path: &path[1..],
                })),
                Scalar::Op(OpDef::Delete(delete_op)) => Some(Box::new(DeleteMethod {
                    subject: self,
                    method: delete_op.clone(),
                    path: &path[1..],
                })),
                other => other.route(&path[1..]),
            }
        } else {
            debug!(
                "{} not found in instance prototype (contents: {}), routing to parent",
                &path[0],
                Tuple::<&Id>::from_iter(self.proto().keys()),
            );

            self.parent().route(path)
        }
    }
}

async fn call_method<T: Instance + Route>(
    txn: &Txn,
    subject: &InstanceExt<T>,
    path: &[PathSegment],
    context: Map<State>,
    form: Vec<(Id, Scalar)>,
) -> TCResult<State> {
    debug!(
        "call method with form {:?}",
        form.iter()
            .map(|(id, s)| format!("{}: {}", id, s))
            .collect::<Vec<String>>()
            .join("\n")
    );

    if !path.is_empty() {
        return Err(TCError::not_found(TCPath::from(path)));
    }

    let capture = if let Some((capture, _)) = form.last() {
        capture.clone()
    } else {
        return Ok(State::default());
    };

    Executor::with_context(txn, Some(subject), context.into(), form)
        .capture(capture)
        .await
}
