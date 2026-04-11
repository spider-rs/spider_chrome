use futures_util::future::{Fuse, FusedFuture};
use futures_util::FutureExt;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

use crate::handler::commandfuture::CommandFuture;
use crate::handler::target::TargetMessage;
use crate::handler::target_message_future::TargetMessageFuture;
use crate::{ArcHttpRequest, Result};
use chromiumoxide_types::Command;

type ArcRequest = ArcHttpRequest;

pin_project! {
    pub struct HttpFuture<T: Command> {
        #[pin]
        command: Fuse<CommandFuture<T>>,
        #[pin]
        navigation: TargetMessageFuture<ArcHttpRequest>,
    }
}

impl<T: Command> HttpFuture<T> {
    pub fn new(
        sender: mpsc::Sender<TargetMessage>,
        command: CommandFuture<T>,
        request_timeout: std::time::Duration,
    ) -> Self {
        Self {
            command: command.fuse(),
            navigation: TargetMessageFuture::<T>::wait_for_navigation(sender, request_timeout),
        }
    }
}

impl<T> Future for HttpFuture<T>
where
    T: Command,
{
    type Output = Result<ArcRequest>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // 1. First complete command request future
        // 2. Switch polls navigation
        if this.command.is_terminated() {
            this.navigation.poll(cx)
        } else {
            match this.command.poll(cx) {
                Poll::Ready(Ok(_command_response)) => {
                    // Command succeeded — reset the navigation timer so it
                    // gets a full request_timeout from NOW, not from when
                    // HttpFuture was constructed.
                    this.navigation.reset_deadline();
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}
