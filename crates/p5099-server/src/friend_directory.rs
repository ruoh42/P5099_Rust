use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use p5099_core::{
    nickname::canonical_nickname_key,
    nm_social::{
        SOCIAL_ALREADY_FRIEND, SOCIAL_ALREADY_REQUEST, SOCIAL_MAX_REQUEST_LIMIT,
        SOCIAL_NOT_EXIST_USER, SOCIAL_OK, SOCIAL_REQUEST_ERROR,
    },
};
use serde::{Deserialize, Serialize};

pub(crate) const FRIEND_DIRECTORY_FILE_NAME: &str = "Friends.json";
const FIRST_SOCIAL_ID: u64 = 1_000_001;
const MAX_FRIENDS: usize = 200;
const MAX_PENDING_REQUESTS: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FriendBook {
    next_id: u64,
    #[serde(default)]
    users: Vec<StoredUser>,
    #[serde(default)]
    friends: Vec<[String; 2]>,
    #[serde(default)]
    requests: Vec<StoredRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredUser {
    nickname: String,
    id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StoredRequest {
    from: String,
    to: String,
    message: String,
}

fn same(left: &str, right: &str) -> bool {
    canonical_nickname_key(left) == canonical_nickname_key(right)
}

impl FriendBook {
    fn id_of(&mut self, nickname: &str) -> (u64, bool) {
        if let Some(user) = self
            .users
            .iter()
            .find(|user| same(&user.nickname, nickname))
        {
            return (user.id, false);
        }
        let id = self.next_id.max(FIRST_SOCIAL_ID);
        self.next_id = id + 1;
        self.users.push(StoredUser {
            nickname: nickname.to_owned(),
            id,
        });
        (id, true)
    }

    fn nickname_of(&self, id: u64) -> Option<String> {
        self.users
            .iter()
            .find(|user| user.id == id)
            .map(|user| user.nickname.clone())
    }

    fn are_friends(&self, left: &str, right: &str) -> bool {
        self.friends
            .iter()
            .any(|[a, b]| (same(a, left) && same(b, right)) || (same(a, right) && same(b, left)))
    }

    fn friend_count(&self, nickname: &str) -> usize {
        self.friends
            .iter()
            .filter(|[a, b]| same(a, nickname) || same(b, nickname))
            .count()
    }

    fn remove_request(&mut self, from: &str, to: &str) -> bool {
        let before = self.requests.len();
        self.requests
            .retain(|request| !(same(&request.from, from) && same(&request.to, to)));
        self.requests.len() != before
    }
}

#[derive(Debug)]
pub(crate) struct FriendDirectory {
    path: Option<PathBuf>,
    book: Mutex<FriendBook>,
}

impl FriendDirectory {
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            path: None,
            book: Mutex::new(FriendBook::default()),
        }
    }

    pub(crate) fn open(profile_root: &Path) -> Self {
        let path = profile_root.join(FRIEND_DIRECTORY_FILE_NAME);
        let book = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                let aside = path.with_extension("json.unreadable");
                tracing::error!(path = %path.display(), %error, "friend directory is unreadable; keeping it aside");
                let _ = fs::rename(&path, &aside);
                FriendBook::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => FriendBook::default(),
            Err(error) => {
                tracing::error!(path = %path.display(), %error, "cannot read the friend directory");
                FriendBook::default()
            }
        };
        Self {
            path: Some(path),
            book: Mutex::new(book),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FriendBook> {
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn persist(&self, book: &FriendBook) {
        let Some(path) = &self.path else {
            return;
        };
        let result = serde_json::to_vec_pretty(book)
            .map_err(io::Error::other)
            .and_then(|bytes| {
                let temporary = path.with_extension("json.tmp");
                fs::write(&temporary, bytes)?;
                fs::rename(&temporary, path)
            });
        if let Err(error) = result {
            tracing::error!(path = %path.display(), %error, "failed to save the friend directory");
        }
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut FriendBook) -> T) -> T {
        let mut book = self.lock();
        let before = book.clone();
        let value = change(&mut book);
        if *book != before {
            let snapshot = book.clone();
            drop(book);
            self.persist(&snapshot);
        }
        value
    }

    pub(crate) fn id_of(&self, nickname: &str) -> u64 {
        self.mutate(|book| book.id_of(nickname).0)
    }

    pub(crate) fn friends_of(&self, nickname: &str) -> Vec<(u64, String)> {
        self.mutate(|book| {
            let names = book
                .friends
                .iter()
                .filter_map(|[a, b]| {
                    if same(a, nickname) {
                        Some(b.clone())
                    } else if same(b, nickname) {
                        Some(a.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            names
                .into_iter()
                .map(|name| (book.id_of(&name).0, name))
                .collect()
        })
    }

    pub(crate) fn received(&self, nickname: &str) -> Vec<(u64, String, String)> {
        self.mutate(|book| {
            let pending = book
                .requests
                .iter()
                .filter(|request| same(&request.to, nickname))
                .map(|request| (request.from.clone(), request.message.clone()))
                .collect::<Vec<_>>();
            pending
                .into_iter()
                .map(|(name, message)| (book.id_of(&name).0, name, message))
                .collect()
        })
    }

    pub(crate) fn sent(&self, nickname: &str) -> Vec<(u64, String, String)> {
        self.mutate(|book| {
            let pending = book
                .requests
                .iter()
                .filter(|request| same(&request.from, nickname))
                .map(|request| (request.to.clone(), request.message.clone()))
                .collect::<Vec<_>>();
            pending
                .into_iter()
                .map(|(name, message)| (book.id_of(&name).0, name, message))
                .collect()
        })
    }

    pub(crate) fn send(&self, from: &str, target: &str, target_exists: bool) -> u32 {
        if !target_exists || target.trim().is_empty() {
            return SOCIAL_NOT_EXIST_USER;
        }
        if same(from, target) {
            return SOCIAL_REQUEST_ERROR;
        }
        self.mutate(|book| {
            if book.are_friends(from, target) {
                return SOCIAL_ALREADY_FRIEND;
            }
            if book.remove_request(target, from) {
                book.friends.push([from.to_owned(), target.to_owned()]);
                return SOCIAL_OK;
            }
            if book
                .requests
                .iter()
                .any(|request| same(&request.from, from) && same(&request.to, target))
            {
                return SOCIAL_ALREADY_REQUEST;
            }
            if book.friend_count(from) >= MAX_FRIENDS
                || book
                    .requests
                    .iter()
                    .filter(|request| same(&request.from, from))
                    .count()
                    >= MAX_PENDING_REQUESTS
            {
                return SOCIAL_MAX_REQUEST_LIMIT;
            }
            book.id_of(target);
            book.requests.push(StoredRequest {
                from: from.to_owned(),
                to: target.to_owned(),
                message: String::new(),
            });
            SOCIAL_OK
        })
    }

    pub(crate) fn accept(&self, me: &str, requester_id: u64) -> u32 {
        self.mutate(|book| {
            let Some(known) = book.nickname_of(requester_id) else {
                return SOCIAL_NOT_EXIST_USER;
            };
            let Some(requester) = book
                .requests
                .iter()
                .find(|request| same(&request.from, &known) && same(&request.to, me))
                .map(|request| request.from.clone())
            else {
                return SOCIAL_REQUEST_ERROR;
            };
            book.remove_request(&requester, me);
            if !book.are_friends(&requester, me) {
                book.friends.push([requester, me.to_owned()]);
            }
            SOCIAL_OK
        })
    }

    pub(crate) fn reject(&self, me: &str, requester_id: u64) -> u32 {
        self.mutate(|book| match book.nickname_of(requester_id) {
            Some(requester) if book.remove_request(&requester, me) => SOCIAL_OK,
            Some(_) => SOCIAL_REQUEST_ERROR,
            None => SOCIAL_NOT_EXIST_USER,
        })
    }

    pub(crate) fn cancel(&self, me: &str, target_id: u64) -> u32 {
        self.mutate(|book| match book.nickname_of(target_id) {
            Some(target) if book.remove_request(me, &target) => SOCIAL_OK,
            Some(_) => SOCIAL_REQUEST_ERROR,
            None => SOCIAL_NOT_EXIST_USER,
        })
    }

    pub(crate) fn delete(&self, me: &str, friend_id: u64) -> u32 {
        self.mutate(|book| {
            let Some(friend) = book.nickname_of(friend_id) else {
                return SOCIAL_NOT_EXIST_USER;
            };
            let before = book.friends.len();
            book.friends.retain(|[a, b]| {
                !((same(a, me) && same(b, &friend)) || (same(a, &friend) && same(b, me)))
            });
            if book.friends.len() == before {
                SOCIAL_REQUEST_ERROR
            } else {
                SOCIAL_OK
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_become_friendships_and_can_be_undone() {
        let friends = FriendDirectory::in_memory();
        assert_eq!(friends.send("Alice", "Bob", false), SOCIAL_NOT_EXIST_USER);
        assert_eq!(friends.send("Alice", "alice", true), SOCIAL_REQUEST_ERROR);
        assert_eq!(friends.send("Alice", "Bob", true), SOCIAL_OK);
        assert_eq!(friends.send("Alice", "bob", true), SOCIAL_ALREADY_REQUEST);

        let alice = friends.id_of("Alice");
        assert_eq!(friends.id_of("alice"), alice);
        assert_eq!(friends.sent("Alice").len(), 1);
        let received = friends.received("Bob");
        assert_eq!((received[0].0, received[0].1.as_str()), (alice, "Alice"));

        assert_eq!(friends.accept("Bob", alice), SOCIAL_OK);
        assert!(friends.received("Bob").is_empty());
        assert_eq!(friends.friends_of("Bob"), vec![(alice, "Alice".to_owned())]);
        assert_eq!(friends.send("Bob", "Alice", true), SOCIAL_ALREADY_FRIEND);

        let bob = friends.id_of("Bob");
        assert_eq!(friends.delete("Alice", bob), SOCIAL_OK);
        assert!(friends.friends_of("Alice").is_empty());
        assert_eq!(friends.delete("Alice", bob), SOCIAL_REQUEST_ERROR);
    }

    #[test]
    fn crossing_requests_make_friends_and_ids_persist() {
        let root = tempfile::tempdir().unwrap();
        let friends = FriendDirectory::open(root.path());
        assert_eq!(friends.send("Alice", "Bob", true), SOCIAL_OK);
        assert_eq!(friends.send("Bob", "Alice", true), SOCIAL_OK);
        assert_eq!(friends.friends_of("Alice").len(), 1);
        let carol = friends.id_of("Carol");
        assert_eq!(friends.send("Carol", "Alice", true), SOCIAL_OK);
        assert_eq!(friends.reject("Alice", carol), SOCIAL_OK);
        let alice = friends.id_of("Alice");
        drop(friends);

        let reopened = FriendDirectory::open(root.path());
        assert_eq!(reopened.id_of("alice"), alice);
        assert_eq!(reopened.friends_of("Bob").len(), 1);
        assert!(reopened.received("Alice").is_empty());
    }
}
