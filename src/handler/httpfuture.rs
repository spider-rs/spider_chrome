use crate::handler::commandfuture::CommandFuture;
use crate::handler::sender::PageSender;
use crate::handler::target_message_future::TargetMessageFuture;
use crate::{ArcHttpRequest, Result};
use chromiumoxide_types::Command;
use futures_util::future::{Fuse, FusedFuture};
use futures_util::FutureExt;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

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
        sender: PageSender,
        command: CommandFuture<T>,
        request_timeout: std::time::Duration,
    ) -> Self {
        Self {
            command: command.fuse(),
            navigation: TargetMessageFuture::<T>::wait_for_navigation(sender, request_timeout),
        }
    }

    /// Like `new` but resolves on `DOMContentLoaded` instead of `load`.
    /// Does not wait for subresources (images, fonts, XHRs) — significantly
    /// faster through slow proxies.
    pub fn new_dom_content_loaded(
        sender: PageSender,
        command: CommandFuture<T>,
        request_timeout: std::time::Duration,
    ) -> Self {
        Self {
            command: command.fuse(),
            navigation: TargetMessageFuture::<T>::wait_for_dom_content_loaded(
                sender,
                request_timeout,
            ),
        }
    }
}

impl<T> Future for HttpFuture<T>
where
    T: Command,
{
    type Output = Result<ArcRequest>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        // 1. First complete command request future
        // 2. Switch polls navigation
        if this.command.is_terminated() {
            this.navigation.poll(cx)
        } else {
            match this.command.poll(cx) {
                Poll::Ready(Ok(_command_response)) => {
                    // Command succeeded — reset the navigation timer so it
                    // gets a full request_timeout from NOW, not from when
                    // HttpFuture was constructed, then immediately start
                    // polling navigation (avoids a full wake round-trip).
                    this.navigation.as_mut().reset_deadline();
                    this.navigation.poll(cx)
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}
