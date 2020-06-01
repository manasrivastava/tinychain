use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::lock::Mutex;

use crate::error;
use crate::transaction::TxnId;
use crate::value::link::{PathSegment, TCPath};
use crate::value::TCResult;

use super::store::Store;

enum DirEntry {
    Dir(Arc<Dir>),
    Store(Arc<Store>),
}

impl fmt::Display for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DirEntry::Dir(_) => write!(f, "(directory)"),
            DirEntry::Store(_) => write!(f, "(block store)"),
        }
    }
}

struct DirState {
    children: HashMap<PathSegment, DirEntry>,
    txn_cache: HashMap<TxnId, HashMap<PathSegment, DirEntry>>,
}

impl DirState {
    fn new() -> DirState {
        DirState {
            children: HashMap::new(),
            txn_cache: HashMap::new(),
        }
    }

    async fn get_dir(&self, txn_id: &TxnId, name: &PathSegment) -> TCResult<Option<Arc<Dir>>> {
        if let Some(Some(entry)) = self.txn_cache.get(txn_id).map(|data| data.get(name)) {
            match entry {
                DirEntry::Dir(dir) => Ok(Some(dir.clone())),
                other => Err(error::bad_request("Not a directory", other)),
            }
        } else if let Some(entry) = self.children.get(name) {
            match entry {
                DirEntry::Dir(dir) => Ok(Some(dir.clone())),
                other => Err(error::bad_request("Not a directory", other)),
            }
        } else {
            Ok(None)
        }
    }
}

pub struct Dir {
    context: PathBuf,
    state: Mutex<DirState>,
}

impl Dir {
    pub fn new(mount_point: PathBuf) -> Arc<Dir> {
        Arc::new(Dir {
            context: mount_point,
            state: Mutex::new(DirState::new()),
        })
    }

    pub fn create_dir<'a>(
        &'a self,
        txn_id: TxnId,
        path: TCPath,
    ) -> BoxFuture<'a, TCResult<Arc<Dir>>> {
        Box::pin(async move {
            if path.is_empty() {
                Err(error::bad_request("Not a valid directory name", path))
            } else if path.len() == 1 {
                let path = path[0].clone();
                let mut state = self.state.lock().await;
                if state.children.contains_key(&path) {
                    Err(error::bad_request("Tried to create a new directory but there is already an entry at this path", &path))
                } else {
                    let txn_data = state.txn_cache.entry(txn_id).or_insert(HashMap::new());
                    if txn_data.contains_key(&path) {
                        Err(error::bad_request(
                            "Tried to create the same directory twice",
                            &path,
                        ))
                    } else {
                        let dir = Dir::new(self.fs_path(&path));
                        txn_data.insert(path, DirEntry::Dir(dir.clone()));
                        Ok(dir)
                    }
                }
            } else {
                let dir = self
                    .get_or_create_dir(&txn_id, path[0].clone().into())
                    .await?;
                dir.create_dir(txn_id, path.slice_from(1)).await
            }
        })
    }

    pub fn get_dir<'a>(
        &'a self,
        txn_id: &'a TxnId,
        path: TCPath,
    ) -> BoxFuture<'a, TCResult<Arc<Dir>>> {
        Box::pin(async move {
            if path.is_empty() {
                Err(error::bad_request("Not a valid directory name", path))
            } else {
                if let Some(dir) = self.state.lock().await.get_dir(txn_id, &path[0]).await? {
                    if path.len() == 1 {
                        Ok(dir)
                    } else {
                        dir.get_dir(txn_id, path.slice_from(1)).await
                    }
                } else {
                    Err(error::not_found(path))
                }
            }
        })
    }

    pub fn get_or_create_dir<'a>(
        &'a self,
        txn_id: &'a TxnId,
        path: TCPath,
    ) -> BoxFuture<'a, TCResult<Arc<Dir>>> {
        Box::pin(async move {
            if path.is_empty() {
                Err(error::bad_request("Not a valid directory name", path))
            } else if path.len() == 1 {
                let mut state = self.state.lock().await;
                if let Some(dir) = state.get_dir(txn_id, &path[0]).await? {
                    Ok(dir)
                } else {
                    let dir = Dir::new(self.fs_path(&path[0]));
                    state
                        .txn_cache
                        .get_mut(&txn_id)
                        .unwrap()
                        .insert(path[0].clone(), DirEntry::Dir(dir.clone()));
                    Ok(dir)
                }
            } else {
                self.get_or_create_dir(txn_id, path[0].clone().into())
                    .await?
                    .get_or_create_dir(txn_id, path.slice_from(1))
                    .await
            }
        })
    }

    fn fs_path(&self, name: &PathSegment) -> PathBuf {
        let mut path = self.context.clone();
        path.push(name.to_string());
        path
    }
}
