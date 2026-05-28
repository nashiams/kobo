use kobo_ir::ScenarioProgram;

use crate::core::ScenarioOptions;
use crate::error::Result;

pub(super) fn tokio_support_source(
    _program: &ScenarioProgram,
    _options: &ScenarioOptions,
) -> Result<String> {
    let mut source = String::from(
        r#"
mod tokio {
    pub mod sync {
        pub mod mpsc {
            use std::sync::{mpsc as std_mpsc, Arc, Mutex};

            pub struct Sender<T> {
                inner: std_mpsc::SyncSender<T>,
            }

            pub struct Receiver<T> {
                inner: Arc<Mutex<std_mpsc::Receiver<T>>>,
            }

            pub mod error {
                pub struct SendError<T>(pub T);
                pub enum TrySendError<T> { Full(T), Closed(T) }
            }

            pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
                let (tx, rx) = std_mpsc::sync_channel(capacity);
                (
                    Sender { inner: tx },
                    Receiver { inner: Arc::new(Mutex::new(rx)) },
                )
            }

            impl<T> Clone for Sender<T> {
                fn clone(&self) -> Self {
                    Self { inner: self.inner.clone() }
                }
            }

            impl<T> Sender<T> {
                pub async fn send(&self, value: T) -> Result<(), error::SendError<T>> {
                    self.inner.send(value).map_err(|error| error::SendError(error.0))
                }

                pub fn try_send(&self, value: T) -> Result<(), error::TrySendError<T>> {
                    self.inner.try_send(value).map_err(|error| match error {
                        std_mpsc::TrySendError::Full(value) => error::TrySendError::Full(value),
                        std_mpsc::TrySendError::Disconnected(value) => error::TrySendError::Closed(value),
                    })
                }
            }

            impl<T> Receiver<T> {
                pub async fn recv(&mut self) -> Option<T> {
                    self.inner.lock().ok()?.recv().ok()
                }
            }
        }

        pub mod oneshot {
            use std::sync::mpsc as std_mpsc;

            pub struct Sender<T> {
                inner: Option<std_mpsc::Sender<T>>,
            }

            pub struct Receiver<T> {
                inner: std_mpsc::Receiver<T>,
            }

            pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
                let (tx, rx) = std_mpsc::channel();
                (Sender { inner: Some(tx) }, Receiver { inner: rx })
            }

            impl<T> Sender<T> {
                pub fn send(mut self, value: T) -> Result<(), T> {
                    match self.inner.take() {
                        Some(sender) => sender.send(value).map_err(|error| error.0),
                        None => Err(value),
                    }
                }
            }

            impl<T> Unpin for Receiver<T> {}

            impl<T> std::future::Future for Receiver<T> {
                type Output = Result<T, ()>;

                fn poll(
                    self: std::pin::Pin<&mut Self>,
                    cx: &mut std::task::Context<'_>,
                ) -> std::task::Poll<Self::Output> {
                    match self.get_mut().inner.try_recv() {
                        Ok(value) => std::task::Poll::Ready(Ok(value)),
                        Err(std_mpsc::TryRecvError::Disconnected) => std::task::Poll::Ready(Err(())),
                        Err(std_mpsc::TryRecvError::Empty) => {
                            cx.waker().wake_by_ref();
                            std::task::Poll::Pending
                        }
                    }
                }
            }
        }
    }

    #[derive(Debug)]
    pub struct JoinError;

    enum JoinState<T> {
        Thread(std::thread::JoinHandle<T>),
        Value(T),
    }

    pub struct JoinHandle<T = ()> {
        state: Option<JoinState<T>>,
    }

    impl<T> JoinHandle<T> {
        fn from_thread(handle: std::thread::JoinHandle<T>) -> Self {
            Self { state: Some(JoinState::Thread(handle)) }
        }

        fn from_value(value: T) -> Self {
            Self { state: Some(JoinState::Value(value)) }
        }

        pub fn abort(self) {}
        pub fn detach_with_policy(self) {}
    }

    impl<T> Unpin for JoinHandle<T> {}

    impl<T> std::future::Future for JoinHandle<T> {
        type Output = Result<T, JoinError>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            let this = self.get_mut();
            if matches!(this.state.as_ref(), Some(JoinState::Thread(handle)) if !handle.is_finished()) {
                cx.waker().wake_by_ref();
                return std::task::Poll::Pending;
            }
            match this.state.take().expect("join handle polled after completion") {
                JoinState::Thread(handle) => std::task::Poll::Ready(handle.join().map_err(|_| JoinError)),
                JoinState::Value(value) => std::task::Poll::Ready(Ok(value)),
            }
        }
    }

    pub mod runtime {
        pub struct Handle;
        pub struct Runtime;
        #[derive(Debug)]
        pub struct BuildError;
        pub struct Builder;

        impl Builder {
            pub fn new_current_thread() -> Self { Self }
            pub fn enable_all(self) -> Self { self }
            pub fn build(self) -> Result<Runtime, BuildError> { Ok(Runtime) }
        }

        impl Runtime {
            pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
                crate::__kobo_block_on(future)
            }
        }

        impl Handle {
            pub fn current() -> Self { Self }
            pub fn try_current() -> Result<Self, ()> { Ok(Self) }

            pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
                crate::__kobo_block_on(future)
            }

            pub fn spawn<F>(&self, future: F) -> super::JoinHandle<F::Output>
            where
                F: std::future::Future + Send + 'static,
                F::Output: Send + 'static,
            {
                super::spawn(future)
            }
        }
    }

    pub mod task {
        pub type JoinHandle<T = ()> = super::JoinHandle<T>;

        pub struct LocalSet;

        impl LocalSet {
            pub fn new() -> Self { Self }

            pub async fn run_until<F: std::future::Future>(&self, future: F) -> F::Output {
                future.await
            }
        }

        pub fn spawn_local<F>(future: F) -> JoinHandle<F::Output>
        where
            F: std::future::Future + 'static,
            F::Output: 'static,
        {
"#,
    );
    source.push_str(
        r#"
            let output = crate::__kobo_block_on(future);
            super::JoinHandle::from_value(output)
        }
    }

    pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send + 'static,
    {
"#,
    );
    source.push_str(
        r#"
        JoinHandle::from_thread(std::thread::spawn(move || crate::__kobo_block_on(future)))
    }
}
"#,
    );
    Ok(source)
}
