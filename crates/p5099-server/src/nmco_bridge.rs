use std::{
    collections::HashMap,
    fmt::Write as _,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use p5099_core::{
    nickname::canonical_nickname_key,
    nm_protocol::{BridgeMessage, MAX_NM_OBJECT_BYTES, decode_frame, length_u32, nm_func_name},
    nm_social::{
        ACCEPT_FRIEND_REQUEST, CANCEL_FRIEND_REQUEST, DELETE_GAME_FRIEND, DELETE_SOCIAL_FRIEND,
        GET_BLOCKED_USER_LIST, GET_GAME_FRIEND_LIST, GET_RECEIVED_FRIEND_REQUESTS,
        GET_SENT_FRIEND_REQUESTS, GET_SOCIAL_FRIEND_LIST, REJECT_FRIEND_REQUEST,
        SEND_FRIEND_REQUEST, SOCIAL_REQUEST_ERROR, SocialUser, code_reply, empty_list_reply,
        friend_list_reply, parse_send_request, parse_target_id, request_list_reply, success_reply,
    },
};
use p5099_profile::ProfileStore;

use crate::connection_limit::ConnectionLimit;
use crate::{account_directory::AccountDirectory, friend_directory::FriendDirectory};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};

const MAX_BRIDGE_MESSAGE: usize = MAX_NM_OBJECT_BYTES + 64;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 512;
const MAX_CONNECTIONS_PER_ADDRESS: usize = 16;
const LOGGED_OBJECT_BYTES: usize = 512;

pub(crate) struct FriendBridge {
    friends: FriendDirectory,
    profiles: ProfileStore,
    accounts: Option<Arc<AccountDirectory>>,
    online: Mutex<HashMap<String, usize>>,
}

impl FriendBridge {
    pub(crate) fn new(profile_root: PathBuf, accounts: Option<Arc<AccountDirectory>>) -> Self {
        Self {
            friends: FriendDirectory::open(&profile_root),
            profiles: ProfileStore::new(profile_root),
            accounts,
            online: Mutex::new(HashMap::new()),
        }
    }

    fn exists(&self, nickname: &str) -> bool {
        self.profiles.profile_exists(nickname).unwrap_or(false)
    }

    fn is_online(&self, nickname: &str) -> bool {
        self.online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&canonical_nickname_key(nickname))
            .is_some_and(|count| *count > 0)
    }

    fn set_online(&self, nickname: &str, connected: bool) {
        let mut online = self
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = canonical_nickname_key(nickname);
        let count = online.entry(key.clone()).or_default();
        if connected {
            *count += 1;
        } else {
            *count = count.saturating_sub(1);
            if *count == 0 {
                online.remove(&key);
            }
        }
    }

    fn user(id: u64, nickname: String) -> SocialUser {
        SocialUser { id, nickname }
    }

    fn handle(&self, me: &str, code: u32, request: &[u8]) -> Option<Vec<u8>> {
        Some(match code {
            GET_SOCIAL_FRIEND_LIST | GET_GAME_FRIEND_LIST => {
                let friends = self
                    .friends
                    .friends_of(me)
                    .into_iter()
                    .map(|(id, name)| {
                        let online = self.is_online(&name);
                        (Self::user(id, name), online)
                    })
                    .collect::<Vec<_>>();
                friend_list_reply(request, &friends)
            }
            GET_RECEIVED_FRIEND_REQUESTS | GET_SENT_FRIEND_REQUESTS => {
                let pending = if code == GET_RECEIVED_FRIEND_REQUESTS {
                    self.friends.received(me)
                } else {
                    self.friends.sent(me)
                };
                let requests = pending
                    .into_iter()
                    .map(|(id, name, message)| (Self::user(id, name), message))
                    .collect::<Vec<_>>();
                request_list_reply(request, &requests)
            }
            GET_BLOCKED_USER_LIST => empty_list_reply(request),
            SEND_FRIEND_REQUEST => {
                let result = match parse_send_request(request) {
                    Ok((_, target)) => {
                        let target = target.trim().to_owned();
                        self.friends.send(me, &target, self.exists(&target))
                    }
                    Err(_) => SOCIAL_REQUEST_ERROR,
                };
                tracing::info!(nickname = me, result, "friend request sent");
                code_reply(request, result)
            }
            ACCEPT_FRIEND_REQUEST
            | REJECT_FRIEND_REQUEST
            | CANCEL_FRIEND_REQUEST
            | DELETE_GAME_FRIEND
            | DELETE_SOCIAL_FRIEND => {
                let result = match parse_target_id(request) {
                    Ok(id) => match code {
                        ACCEPT_FRIEND_REQUEST => self.friends.accept(me, id),
                        REJECT_FRIEND_REQUEST => self.friends.reject(me, id),
                        CANCEL_FRIEND_REQUEST => self.friends.cancel(me, id),
                        _ => self.friends.delete(me, id),
                    },
                    Err(_) => SOCIAL_REQUEST_ERROR,
                };
                tracing::info!(
                    nickname = me,
                    function = nm_func_name(code).unwrap_or("?"),
                    result,
                    "friend action"
                );
                code_reply(request, result)
            }
            _ => return None,
        })
    }
}

pub(crate) fn spawn_nmco_bridge(
    address: SocketAddr,
    profile_root: PathBuf,
    accounts: Option<Arc<AccountDirectory>>,
    shutdown: watch::Receiver<bool>,
) {
    let bridge = Arc::new(FriendBridge::new(profile_root, accounts));
    tokio::spawn(async move {
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(%address, %error, "NMCO messenger bridge disabled");
                return;
            }
        };
        tracing::info!(%address, "NMCO messenger bridge listening");
        accept_loop(listener, bridge, shutdown).await;
    });
}

async fn accept_loop(
    listener: TcpListener,
    bridge: Arc<FriendBridge>,
    mut shutdown: watch::Receiver<bool>,
) {
    let limit = ConnectionLimit::new(MAX_CONNECTIONS, MAX_CONNECTIONS_PER_ADDRESS);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let Some(permit) = limit.try_admit(peer.ip()) else {
                        tracing::debug!(%peer, "NMCO bridge connection refused: too many connections");
                        continue;
                    };
                    let bridge = Arc::clone(&bridge);
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Err(error) = serve_connection(stream, peer, &bridge).await {
                            tracing::debug!(%peer, %error, "NMCO bridge connection ended");
                        }
                    });
                }
                Err(error) => tracing::warn!(%error, "NMCO bridge accept failed"),
            },
        }
    }
}

async fn read_message(stream: &mut TcpStream) -> std::io::Result<BridgeMessage> {
    let length = stream.read_u32_le().await? as usize;
    if length == 0 || length > MAX_BRIDGE_MESSAGE {
        return Err(std::io::Error::other(format!(
            "bridge message of {length} bytes"
        )));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await?;
    BridgeMessage::decode(&body).map_err(std::io::Error::other)
}

async fn write_message(stream: &mut TcpStream, message: &BridgeMessage) -> std::io::Result<()> {
    let body = message.encode();
    let mut packet = Vec::with_capacity(body.len() + 4);
    packet.extend_from_slice(&length_u32(body.len()).to_le_bytes());
    packet.extend_from_slice(&body);
    stream.write_all(&packet).await
}

async fn serve_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    bridge: &FriendBridge,
) -> std::io::Result<()> {
    let hello = tokio::time::timeout(HELLO_TIMEOUT, read_message(&mut stream))
        .await
        .map_err(|_| std::io::Error::other("no hello"))??;
    let BridgeMessage::Hello {
        protocol,
        nickname,
        ticket,
    } = hello
    else {
        return Err(std::io::Error::other("first bridge message is not a hello"));
    };
    let nickname = match &bridge.accounts {
        Some(accounts) => match accounts.redeem(&nickname, &ticket) {
            Some(grant) => grant.nickname,
            None => {
                return Err(std::io::Error::other(format!(
                    "no valid launcher ticket for {nickname}"
                )));
            }
        },
        None => nickname,
    };
    if !bridge.exists(&nickname) {
        return Err(std::io::Error::other(format!("no profile for {nickname}")));
    }
    let social_id = bridge.friends.id_of(&nickname);
    tracing::info!(%peer, protocol, nickname, social_id, "friend bridge connected");
    bridge.set_online(&nickname, true);
    let result = serve_calls(&mut stream, bridge, &nickname).await;
    bridge.set_online(&nickname, false);
    result
}

async fn serve_calls(
    stream: &mut TcpStream,
    bridge: &FriendBridge,
    nickname: &str,
) -> std::io::Result<()> {
    loop {
        let message = read_message(stream).await?;
        let BridgeMessage::Call {
            sequence,
            code,
            object,
        } = message
        else {
            continue;
        };
        let (result, reply) = match decode_frame(&object) {
            Ok((request, _)) => {
                log_call(nickname, code, &request);
                let reply = bridge
                    .handle(nickname, code, &request)
                    .unwrap_or_else(|| success_reply(&request));
                (1, reply)
            }
            Err(error) => {
                tracing::warn!(nickname, code, %error, "undecodable NMCO call");
                (0, Vec::new())
            }
        };
        write_message(
            stream,
            &BridgeMessage::Reply {
                sequence,
                result,
                object: reply,
            },
        )
        .await?;
    }
}

fn log_call(nickname: &str, code: u32, request: &[u8]) {
    let mut hex = String::with_capacity(LOGGED_OBJECT_BYTES * 2);
    for byte in request.iter().take(LOGGED_OBJECT_BYTES) {
        let _ = write!(hex, "{byte:02X}");
    }
    tracing::debug!(
        nickname,
        code = format_args!("0x{code:04X}"),
        function = nm_func_name(code).unwrap_or("?"),
        length = request.len(),
        object = hex,
        "NMCO call"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use p5099_core::{
        nm_protocol::{
            NM_FUNC_CLASS, NmReader, NmWriter, SCRAMBLE_CODEC_MAGIC, SCRAMBLE_CODEC_TRAILER,
            scramble_encode,
        },
        nm_social::{SOCIAL_NOT_EXIST_USER, SOCIAL_OK},
    };
    use p5099_profile::Profile;

    fn scrambled_frame(object: &[u8]) -> Vec<u8> {
        let payload = scramble_encode(object, 0x55AA_1234);
        let mut frame = SCRAMBLE_CODEC_MAGIC.to_le_bytes().to_vec();
        frame.extend_from_slice(&length_u32(payload.len()).to_le_bytes());
        frame.extend_from_slice(&payload);
        frame.extend_from_slice(&SCRAMBLE_CODEC_TRAILER.to_le_bytes());
        frame
    }

    fn request(fields: impl FnOnce(&mut NmWriter)) -> Vec<u8> {
        let mut writer = NmWriter::new();
        writer.begin(NM_FUNC_CLASS, 1, 1).u32(0);
        fields(&mut writer);
        writer.end();
        writer.into_bytes()
    }

    struct Client {
        stream: TcpStream,
        sequence: u32,
    }

    impl Client {
        async fn connect(address: SocketAddr, nickname: &str) -> Self {
            let mut stream = TcpStream::connect(address).await.unwrap();
            write_message(
                &mut stream,
                &BridgeMessage::Hello {
                    protocol: 1,
                    nickname: nickname.to_owned(),
                    ticket: String::new(),
                },
            )
            .await
            .unwrap();
            Self {
                stream,
                sequence: 0,
            }
        }

        async fn call(&mut self, code: u32, object: Vec<u8>) -> Vec<u8> {
            self.sequence += 1;
            write_message(
                &mut self.stream,
                &BridgeMessage::Call {
                    sequence: self.sequence,
                    code,
                    object: scrambled_frame(&object),
                },
            )
            .await
            .unwrap();
            let BridgeMessage::Reply {
                sequence,
                result,
                object,
            } = read_message(&mut self.stream).await.unwrap()
            else {
                panic!("expected a reply");
            };
            assert_eq!((sequence, result), (self.sequence, 1));
            let mut reader = NmReader::new(&object);
            assert_eq!(reader.header().unwrap().class, NM_FUNC_CLASS);
            assert_eq!(reader.u32().unwrap(), 1);
            reader.remaining().to_vec()
        }
    }

    fn u32_at(bytes: &[u8]) -> u32 {
        NmReader::new(bytes).u32().unwrap()
    }

    #[tokio::test]
    async fn riders_befriend_each_other_through_the_bridge() {
        let root = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(root.path());
        for nickname in ["Alice", "Bob"] {
            store.save(nickname, &Profile::default()).unwrap();
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (_shutdown_sender, shutdown) = watch::channel(false);
        let bridge = Arc::new(FriendBridge::new(root.path().to_owned(), None));
        tokio::spawn(accept_loop(listener, Arc::clone(&bridge), shutdown));

        let mut alice = Client::connect(address, "Alice").await;
        let mut bob = Client::connect(address, "Bob").await;

        assert!(
            alice
                .call(
                    0x1111,
                    request(|writer| {
                        writer.u32(0).u32(0);
                    })
                )
                .await
                .is_empty()
        );

        let ghost = alice
            .call(
                SEND_FRIEND_REQUEST,
                request(|writer| {
                    writer.u64(0).wstr("Nobody");
                }),
            )
            .await;
        assert_eq!(u32_at(&ghost), SOCIAL_NOT_EXIST_USER);
        let sent = alice
            .call(
                SEND_FRIEND_REQUEST,
                request(|writer| {
                    writer.u64(0).wstr("Bob");
                }),
            )
            .await;
        assert_eq!(u32_at(&sent), SOCIAL_OK);

        let received = bob
            .call(GET_RECEIVED_FRIEND_REQUESTS, request(|_| {}))
            .await;
        let mut reader = NmReader::new(&received);
        assert_eq!(reader.u32().unwrap(), 1);
        reader.header().unwrap();
        reader.header().unwrap();
        let alice_id = reader.u64().unwrap();
        reader.wstr().unwrap();
        assert_eq!(reader.wstr().unwrap(), "Alice");

        let accepted = bob
            .call(
                ACCEPT_FRIEND_REQUEST,
                request(|writer| {
                    writer.u64(alice_id);
                }),
            )
            .await;
        assert_eq!(u32_at(&accepted), SOCIAL_OK);

        let list = bob.call(GET_GAME_FRIEND_LIST, request(|_| {})).await;
        let mut reader = NmReader::new(&list);
        assert_eq!(reader.u32().unwrap(), 1);
        reader.header().unwrap();
        reader.header().unwrap();
        assert_eq!(reader.u64().unwrap(), alice_id);
        reader.wstr().unwrap();
        assert_eq!(reader.wstr().unwrap(), "Alice");
        reader.wstr().unwrap();
        reader.header().unwrap();
        reader.u32().unwrap();
        reader.wstr().unwrap();
        assert_eq!(reader.u8().unwrap(), 1, "Alice's bridge is connected");

        drop(alice);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!bridge.is_online("Alice"));
    }
}
