use {
    crate::{
        config::ConfigGrpc,
        filters::Filter,
        prom::CONNECTIONS_TOTAL,
        proto::{
            geyser_server::{Geyser, GeyserServer},
            subscribe_update::UpdateOneof,
            SubscribeRequest, SubscribeUpdate, SubscribeUpdateAccount, SubscribeUpdateAccountInfo,
            SubscribeUpdateBlock, SubscribeUpdateBlockMeta, SubscribeUpdatePing,
            SubscribeUpdateSlot, SubscribeUpdateSlotStatus, SubscribeUpdateTransaction,
            SubscribeUpdateTransactionInfo,
        },
    },
    log::*,
    solana_geyser_plugin_interface::geyser_plugin_interface::{
        ReplicaAccountInfoV2, ReplicaBlockInfoV2, ReplicaTransactionInfoV2, SlotStatus,
    },
    solana_sdk::{
        clock::UnixTimestamp, pubkey::Pubkey, signature::Signature,
        transaction::SanitizedTransaction,
    },
    solana_transaction_status::{Reward, TransactionStatusMeta},
    std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    },
    tokio::{
        sync::{mpsc, oneshot},
        time::sleep,
    },
    tokio_stream::wrappers::ReceiverStream,
    tonic::{
        codec::CompressionEncoding,
        transport::server::{Server, TcpIncoming},
        Request, Response, Result as TonicResult, Status, Streaming,
    },
    tonic_health::server::health_reporter,
};

#[derive(Debug)]
pub struct MessageAccountInfo {
    pub pubkey: Pubkey,
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data: Vec<u8>,
    pub write_version: u64,
    pub txn_signature: Option<Signature>,
}

#[derive(Debug)]
pub struct MessageAccount {
    pub account: MessageAccountInfo,
    pub slot: u64,
    pub is_startup: bool,
}

impl<'a> From<(&'a ReplicaAccountInfoV2<'a>, u64, bool)> for MessageAccount {
    fn from((account, slot, is_startup): (&'a ReplicaAccountInfoV2<'a>, u64, bool)) -> Self {
        Self {
            account: MessageAccountInfo {
                pubkey: Pubkey::try_from(account.pubkey).expect("valid Pubkey"),
                lamports: account.lamports,
                owner: Pubkey::try_from(account.owner).expect("valid Pubkey"),
                executable: account.executable,
                rent_epoch: account.rent_epoch,
                data: account.data.into(),
                write_version: account.write_version,
                txn_signature: account.txn_signature.cloned(),
            },
            slot,
            is_startup,
        }
    }
}

#[derive(Debug)]
pub struct MessageSlot {
    pub slot: u64,
    pub parent: Option<u64>,
    pub status: SubscribeUpdateSlotStatus,
}

impl From<(u64, Option<u64>, SlotStatus)> for MessageSlot {
    fn from((slot, parent, status): (u64, Option<u64>, SlotStatus)) -> Self {
        Self {
            slot,
            parent,
            status: match status {
                SlotStatus::Processed => SubscribeUpdateSlotStatus::Processed,
                SlotStatus::Confirmed => SubscribeUpdateSlotStatus::Confirmed,
                SlotStatus::Rooted => SubscribeUpdateSlotStatus::Finalized,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageTransactionInfo {
    pub signature: Signature,
    pub is_vote: bool,
    pub transaction: SanitizedTransaction,
    pub meta: TransactionStatusMeta,
    pub index: usize,
}

impl From<&MessageTransactionInfo> for SubscribeUpdateTransactionInfo {
    fn from(tx: &MessageTransactionInfo) -> Self {
        Self {
            signature: tx.signature.as_ref().into(),
            is_vote: tx.is_vote,
            transaction: Some((&tx.transaction).into()),
            meta: Some((&tx.meta).into()),
            index: tx.index as u64,
        }
    }
}

#[derive(Debug)]
pub struct MessageTransaction {
    pub transaction: MessageTransactionInfo,
    pub slot: u64,
}

impl<'a> From<(&'a ReplicaTransactionInfoV2<'a>, u64)> for MessageTransaction {
    fn from((transaction, slot): (&'a ReplicaTransactionInfoV2<'a>, u64)) -> Self {
        Self {
            transaction: MessageTransactionInfo {
                signature: *transaction.signature,
                is_vote: transaction.is_vote,
                transaction: transaction.transaction.clone(),
                meta: transaction.transaction_status_meta.clone(),
                index: transaction.index,
            },
            slot,
        }
    }
}

#[derive(Debug)]
pub struct MessageBlock {
    pub parent_slot: u64,
    pub slot: u64,
    pub parent_blockhash: String,
    pub blockhash: String,
    pub rewards: Vec<Reward>,
    pub block_time: Option<UnixTimestamp>,
    pub block_height: Option<u64>,
    pub transactions: Vec<MessageTransactionInfo>,
}

impl From<(MessageBlockMeta, Vec<MessageTransactionInfo>)> for MessageBlock {
    fn from((blockinfo, transactions): (MessageBlockMeta, Vec<MessageTransactionInfo>)) -> Self {
        Self {
            parent_slot: blockinfo.parent_slot,
            slot: blockinfo.slot,
            blockhash: blockinfo.blockhash,
            parent_blockhash: blockinfo.parent_blockhash,
            rewards: blockinfo.rewards,
            block_time: blockinfo.block_time,
            block_height: blockinfo.block_height,
            transactions,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageBlockMeta {
    pub parent_slot: u64,
    pub slot: u64,
    pub parent_blockhash: String,
    pub blockhash: String,
    pub rewards: Vec<Reward>,
    pub block_time: Option<UnixTimestamp>,
    pub block_height: Option<u64>,
    pub executed_transaction_count: u64,
}

impl<'a> From<&'a ReplicaBlockInfoV2<'a>> for MessageBlockMeta {
    fn from(blockinfo: &'a ReplicaBlockInfoV2<'a>) -> Self {
        Self {
            parent_slot: blockinfo.parent_slot,
            slot: blockinfo.slot,
            parent_blockhash: blockinfo.parent_blockhash.to_string(),
            blockhash: blockinfo.blockhash.to_string(),
            rewards: blockinfo.rewards.into(),
            block_time: blockinfo.block_time,
            block_height: blockinfo.block_height,
            executed_transaction_count: blockinfo.executed_transaction_count,
        }
    }
}

#[derive(Debug)]
pub enum Message {
    Slot(MessageSlot),
    Account(MessageAccount),
    Transaction(MessageTransaction),
    Block(MessageBlock),
    BlockMeta(MessageBlockMeta),
}

impl From<&Message> for UpdateOneof {
    fn from(message: &Message) -> Self {
        match message {
            Message::Slot(message) => UpdateOneof::Slot(SubscribeUpdateSlot {
                slot: message.slot,
                parent: message.parent,
                status: message.status as i32,
            }),
            Message::Account(message) => UpdateOneof::Account(SubscribeUpdateAccount {
                account: Some(SubscribeUpdateAccountInfo {
                    pubkey: message.account.pubkey.as_ref().into(),
                    lamports: message.account.lamports,
                    owner: message.account.owner.as_ref().into(),
                    executable: message.account.executable,
                    rent_epoch: message.account.rent_epoch,
                    data: message.account.data.clone(),
                    write_version: message.account.write_version,
                    txn_signature: message.account.txn_signature.map(|s| s.as_ref().into()),
                }),
                slot: message.slot,
                is_startup: message.is_startup,
            }),
            Message::Transaction(message) => UpdateOneof::Transaction(SubscribeUpdateTransaction {
                transaction: Some((&message.transaction).into()),
                slot: message.slot,
            }),
            Message::Block(message) => UpdateOneof::Block(SubscribeUpdateBlock {
                slot: message.slot,
                blockhash: message.blockhash.clone(),
                rewards: Some(message.rewards.as_slice().into()),
                block_time: message.block_time.map(|v| v.into()),
                block_height: message.block_height.map(|v| v.into()),
                transactions: message.transactions.iter().map(Into::into).collect(),
                parent_slot: message.parent_slot,
                parent_blockhash: message.parent_blockhash.clone(),
            }),
            Message::BlockMeta(message) => UpdateOneof::BlockMeta(SubscribeUpdateBlockMeta {
                slot: message.slot,
                blockhash: message.blockhash.clone(),
                rewards: Some(message.rewards.as_slice().into()),
                block_time: message.block_time.map(|v| v.into()),
                block_height: message.block_height.map(|v| v.into()),
                parent_slot: message.parent_slot,
                parent_blockhash: message.parent_blockhash.clone(),
                executed_transaction_count: message.executed_transaction_count,
            }),
        }
    }
}

#[derive(Debug)]
enum ClientMessage {
    New {
        id: usize,
        filter: Filter,
        stream_tx: mpsc::Sender<TonicResult<SubscribeUpdate>>,
    },
    Update {
        id: usize,
        filter: Filter,
    },
}

#[derive(Debug)]
struct ClientConnection {
    filter: Filter,
    stream_tx: mpsc::Sender<TonicResult<SubscribeUpdate>>,
}

#[derive(Debug)]
pub struct GrpcService {
    config: ConfigGrpc,
    subscribe_id: AtomicUsize,
    new_clients_tx: mpsc::UnboundedSender<ClientMessage>,
}

impl GrpcService {
    pub fn create(
        config: ConfigGrpc,
    ) -> Result<
        (mpsc::UnboundedSender<Message>, oneshot::Sender<()>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Bind service address
        let incoming = TcpIncoming::new(
            config.address,
            true,                          // tcp_nodelay
            Some(Duration::from_secs(20)), // tcp_keepalive
        )?;

        // Create Server
        let (new_clients_tx, new_clients_rx) = mpsc::unbounded_channel();
        let service = GeyserServer::new(Self {
            config,
            subscribe_id: AtomicUsize::new(0),
            new_clients_tx,
        })
        .accept_compressed(CompressionEncoding::Gzip)
        .send_compressed(CompressionEncoding::Gzip);

        // Run filter and send loop
        let (update_channel_tx, update_channel_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move { Self::send_loop(update_channel_rx, new_clients_rx).await });

        // gRPC Health check service
        let (mut health_reporter, health_service) = health_reporter();
        tokio::spawn(async move { health_reporter.set_serving::<GeyserServer<Self>>().await });

        // Run Server
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            Server::builder()
                .http2_keepalive_interval(Some(Duration::from_secs(5)))
                .add_service(health_service)
                .add_service(service)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_rx.await;
                })
                .await
        });

        Ok((update_channel_tx, shutdown_tx))
    }

    async fn send_loop(
        mut update_channel_rx: mpsc::UnboundedReceiver<Message>,
        mut new_clients_rx: mpsc::UnboundedReceiver<ClientMessage>,
    ) {
        let mut clients: HashMap<usize, ClientConnection> = HashMap::new();
        loop {
            tokio::select! {
                Some(message) = update_channel_rx.recv() => {
                    let mut ids_full = vec![];
                    let mut ids_closed = vec![];

                    for (id, client) in clients.iter() {
                        let filters = client.filter.get_filters(&message);
                        if !filters.is_empty() {
                            match client.stream_tx.try_send(Ok(SubscribeUpdate {
                                filters,
                                update_oneof: Some((&message).into()),
                            })) {
                                Ok(()) => {},
                                Err(mpsc::error::TrySendError::Full(_)) => ids_full.push(*id),
                                Err(mpsc::error::TrySendError::Closed(_)) => ids_closed.push(*id),
                            }
                        }
                    }

                    for id in ids_full {
                        if let Some(client) = clients.remove(&id) {
                            tokio::spawn(async move {
                                CONNECTIONS_TOTAL.dec();
                                error!("{}, lagged, close stream", id);
                                let _ = client.stream_tx.send(Err(Status::internal("lagged"))).await;
                            });
                        }
                    }
                    for id in ids_closed {
                        if let Some(_client) = clients.remove(&id) {
                            CONNECTIONS_TOTAL.dec();
                            error!("{}, client closed stream", id);
                        }
                    }
                },
                Some(msg) = new_clients_rx.recv() => {
                    match msg {
                        ClientMessage::New { id, filter, stream_tx } => {
                            info!("{}, add client to receivers", id);
                            clients.insert(id, ClientConnection { filter, stream_tx });
                            CONNECTIONS_TOTAL.inc();
                        }
                        ClientMessage::Update {id,filter} => {
                            if let Some(client) = clients.get_mut(&id) {
                                info!("{}, update client", id);
                                client.filter = filter;
                            }
                        }
                    }
                }
                else => break,
            };
        }
    }
}

#[tonic::async_trait]
impl Geyser for GrpcService {
    type SubscribeStream = ReceiverStream<TonicResult<SubscribeUpdate>>;

    async fn subscribe(
        &self,
        mut request: Request<Streaming<SubscribeRequest>>,
    ) -> TonicResult<Response<Self::SubscribeStream>> {
        let id = self.subscribe_id.fetch_add(1, Ordering::SeqCst);
        info!("{}, new subscriber", id);

        let filter = Filter::new(
            &SubscribeRequest {
                accounts: HashMap::new(),
                slots: HashMap::new(),
                transactions: HashMap::new(),
                blocks: HashMap::new(),
                blocks_meta: HashMap::new(),
            },
            self.config.filters.as_ref(),
        )
        .expect("empty filter");

        let (stream_tx, stream_rx) = mpsc::channel(self.config.channel_capacity);
        if let Err(_error) = self.new_clients_tx.send(ClientMessage::New {
            id,
            filter,
            stream_tx: stream_tx.clone(),
        }) {
            return Err(Status::internal("failed to add client"));
        }

        let ping_stream_tx = stream_tx.clone();
        tokio::spawn(async move {
            loop {
                sleep(Duration::from_secs(10)).await;
                match ping_stream_tx.try_send(Ok(SubscribeUpdate {
                    filters: vec![],
                    update_oneof: Some(UpdateOneof::Ping(SubscribeUpdatePing {})),
                })) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });

        let config_filters_limit = self.config.filters.clone();
        let new_clients_tx = self.new_clients_tx.clone();
        tokio::spawn(async move {
            loop {
                match request.get_mut().message().await {
                    Ok(Some(request)) => {
                        if let Err(error) =
                            match Filter::new(&request, config_filters_limit.as_ref()) {
                                Ok(filter) => {
                                    match new_clients_tx.send(ClientMessage::Update { id, filter })
                                    {
                                        Ok(()) => Ok(()),
                                        Err(error) => Err(error.to_string()),
                                    }
                                }
                                Err(error) => Err(error.to_string()),
                            }
                        {
                            let _ = stream_tx
                                .send(Err(Status::invalid_argument(format!(
                                    "failed to create filter: {}",
                                    error
                                ))))
                                .await;
                        }
                    }
                    Ok(None) => break,
                    Err(_error) => break,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(stream_rx)))
    }
}
