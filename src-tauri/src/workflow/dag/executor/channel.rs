use std::sync::Arc;
use tokio::sync::{oneshot, watch};

#[derive(Clone)]
pub struct SharedChannel<T: Clone + Send + 'static> {
    /// 发送器
    sender: Arc<tokio::sync::Mutex<Option<oneshot::Sender<T>>>>,
    /// 接收器
    receiver: Arc<tokio::sync::Mutex<Option<oneshot::Receiver<T>>>>,
}

impl<T: Clone + Send + 'static> SharedChannel<T> {
    /// 创建新的共享通道
    pub fn new() -> Self {
        let (tx, rx) = oneshot::channel();
        Self {
            sender: Arc::new(tokio::sync::Mutex::new(Some(tx))),
            receiver: Arc::new(tokio::sync::Mutex::new(Some(rx))),
        }
    }

    /// 发送信号
    pub async fn send(&self, value: T) -> Result<(), T> {
        let mut sender = self.sender.lock().await;

        // 如果发送器已经被使用，创建一个新的通道
        if sender.is_none() {
            let (tx, rx) = oneshot::channel();
            *sender = Some(tx);
            let mut receiver = self.receiver.lock().await;
            *receiver = Some(rx);
        }

        // 尝试发送值
        match sender.take() {
            Some(tx) => {
                if let Err(val) = tx.send(value) {
                    // 发送失败，创建新通道并返回错误
                    let (new_tx, new_rx) = oneshot::channel();
                    *sender = Some(new_tx);
                    let mut receiver = self.receiver.lock().await;
                    *receiver = Some(new_rx);
                    Err(val)
                } else {
                    // 发送成功，创建新通道
                    let (new_tx, new_rx) = oneshot::channel();
                    *sender = Some(new_tx);
                    let mut receiver = self.receiver.lock().await;
                    *receiver = Some(new_rx);
                    Ok(())
                }
            }
            None => {
                // 发送器为空，创建新通道并返回错误
                let (new_tx, new_rx) = oneshot::channel();
                *sender = Some(new_tx);
                let mut receiver = self.receiver.lock().await;
                *receiver = Some(new_rx);
                Err(value)
            }
        }
    }

    /// 接收信号
    pub async fn recv(&self) -> Option<T> {
        let mut receiver = self.receiver.lock().await;

        if let Some(mut rx) = receiver.take() {
            match rx.try_recv() {
                Ok(value) => {
                    // 接收成功，创建新通道
                    let (new_tx, new_rx) = oneshot::channel();
                    *receiver = Some(new_rx);
                    let mut sender = self.sender.lock().await;
                    *sender = Some(new_tx);
                    Some(value)
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    // 通道为空，恢复接收器
                    *receiver = Some(rx);
                    None
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    // 通道已关闭，创建新通道
                    let (new_tx, new_rx) = oneshot::channel();
                    *receiver = Some(new_rx);
                    let mut sender = self.sender.lock().await;
                    *sender = Some(new_tx);
                    None
                }
            }
        } else {
            // 接收器为空，创建新通道
            let (new_tx, new_rx) = oneshot::channel();
            *receiver = Some(new_rx);
            let mut sender = self.sender.lock().await;
            *sender = Some(new_tx);
            None
        }
    }

    /// 检查通道是否已关闭
    pub async fn is_closed(&self) -> bool {
        let sender = self.sender.lock().await;

        if let Some(tx) = sender.as_ref() {
            tx.is_closed()
        } else {
            true
        }
    }
}

/// 观察通道，用于状态变更广播
#[derive(Clone)]
pub struct WatchChannel<T> {
    /// 发送端
    pub tx: watch::Sender<T>,
    /// 接收端
    pub rx: watch::Receiver<T>,
}

impl<T: Clone> WatchChannel<T> {
    /// 创建新的观察通道
    pub fn new(initial_value: T) -> Self {
        let (tx, rx) = watch::channel(initial_value);
        Self { tx, rx }
    }
}
