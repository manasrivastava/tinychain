use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use crate::internal::Dir;
use crate::transaction::TxnId;
use crate::value::link::TCPath;

type Blocks = Box<dyn Stream<Item = Bytes> + Send + Unpin>;
type FileData = (TCPath, Blocks);

#[async_trait]
pub trait Archive {
    async fn copy_from(reader: &mut FileCopier, txn_id: &TxnId, dest: Arc<Dir>) -> Arc<Self>;

    async fn copy_into(&self, txn_id: TxnId, writer: &mut FileCopier);

    async fn inflate(txn_id: &TxnId, archive: Arc<Dir>) -> Arc<Self>;
}

struct SharedState {
    open: bool,
    waker: Option<Waker>,
}

pub struct FileCopier {
    contents: RwLock<VecDeque<FileData>>,
    shared_state: Arc<Mutex<SharedState>>,
}

impl FileCopier {
    pub fn open() -> FileCopier {
        FileCopier {
            contents: RwLock::new(VecDeque::new()),
            shared_state: Arc::new(Mutex::new(SharedState {
                open: true,
                waker: None,
            })),
        }
    }

    pub async fn copy<T: Archive>(txn_id: TxnId, state: &T, dest: Arc<Dir>) -> Arc<T> {
        let mut copier = Self::open();
        state.copy_into(txn_id.clone(), &mut copier).await;
        copier.close();
        T::copy_from(&mut copier, &txn_id, dest).await
    }

    pub fn close(&mut self) {
        self.shared_state.lock().unwrap().open = false;
    }

    pub fn write_file(&mut self, path: TCPath, blocks: Blocks) {
        let shared_state = self.shared_state.lock().unwrap();
        if !shared_state.open {
            panic!("Tried to write file to closed FileCopier");
        } else if path.len() != 1 {
            panic!("Tried to write file in subdirectory: {}", path);
        }

        println!("FileCopier::write_file {}", path);

        self.contents.write().unwrap().push_back((path, blocks));
        if let Some(waker) = &shared_state.waker {
            waker.clone().wake();
        }
    }
}

impl Stream for FileCopier {
    type Item = FileData;

    fn poll_next(self: Pin<&mut Self>, cxt: &mut Context) -> Poll<Option<Self::Item>> {
        let mut shared_state = self.shared_state.lock().unwrap();
        if self.contents.read().unwrap().is_empty() {
            if shared_state.open {
                shared_state.waker = Some(cxt.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(None)
            }
        } else {
            let item = self.contents.write().unwrap().pop_front();
            if let Some((path, _)) = &item {
                println!("FileCopier::next ({}, <blocks>)", path);
            }

            Poll::Ready(item)
        }
    }
}