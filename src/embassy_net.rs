use std::{
    future::Future,
    pin::Pin,
    task::{self, Poll},
};

use embassy_net::tcp::TcpReader;
use pin_project::pin_project;
use tcp_reader_read_future::TcpReaderReadFuture;

use crate::LendingAsyncIterator;

#[pin_project(project = TcpReaderLendingAsyncIteratorProj)]
pub struct TcpReaderLendingAsyncIterator<'d, const BUF_SIZE: usize> {
    #[pin]
    status: Status<'d, BUF_SIZE>,
    pos: usize,
}

impl<'d, const BUF_SIZE: usize> LendingAsyncIterator
    for TcpReaderLendingAsyncIterator<'d, BUF_SIZE>
{
    type Item<'a> = Result<&'a [u8], embassy_net::tcp::Error>
    where
        Self: 'a;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut task::Context) -> Poll<Option<Self::Item<'_>>> {
        let mut this = self.as_mut().project();

        match this.status.as_mut().project() {
            TcpReaderOrReadFutureProj::Data { buffer, .. } => {
                if *this.pos > BUF_SIZE / 2 {
                    buffer.copy_within((*this.pos).., 0);
                    *this.pos = 0;
                }

                let TcpReaderOrReadFutureProjOwn::Data { tcp_reader, buffer } =
                    this.status.as_mut().project_replace(Status::Invalid)
                else {
                    unreachable!();
                };

                let pos = *this.pos;
                let future = tcp_reader_read_future::create(tcp_reader, buffer, pos);
                this.status.set(Status::Future(future));

                let TcpReaderOrReadFutureProj::Future(future) = this.status.project() else {
                    unreachable!();
                };
                let (result, tcp_reader, buffer) = task::ready!(future.poll(cx));
                self.handle_future_result(result, tcp_reader, buffer)
            }

            TcpReaderOrReadFutureProj::Future(future) => {
                let (result, tcp_reader, buffer) = task::ready!(future.poll(cx));
                self.handle_future_result(result, tcp_reader, buffer)
            }

            TcpReaderOrReadFutureProj::Invalid => unreachable!(),
        }
    }
}

impl<'d, const BUF_SIZE: usize> TcpReaderLendingAsyncIterator<'d, BUF_SIZE> {
    fn handle_future_result<'a>(
        self: Pin<&'a mut Self>,
        result: Result<usize, embassy_net::tcp::Error>,
        tcp_reader: TcpReader<'d>,
        buffer: [u8; BUF_SIZE],
    ) -> Poll<Option<Result<&'a [u8], embassy_net::tcp::Error>>> {
        let mut this = self.project();
        this.status.set(Status::Data { tcp_reader, buffer });
        let TcpReaderOrReadFutureProj::Data { buffer, .. } = this.status.project() else {
            unreachable!()
        };

        Poll::Ready(
            result
                .map(|bytes_read| {
                    (bytes_read != 0).then(|| {
                        let pos = *this.pos;
                        let out = &buffer[pos..(pos + bytes_read)];
                        *this.pos += bytes_read;

                        out
                    })
                })
                .transpose(),
        )
    }
}

mod tcp_reader_read_future {
    use std::future::Future;

    use embassy_net::tcp::TcpReader;

    pub type TcpReaderReadFuture<'d, const BUF_SIZE: usize> = impl Future<
        Output = (
            Result<usize, embassy_net::tcp::Error>,
            TcpReader<'d>,
            [u8; BUF_SIZE],
        ),
    >;

    pub(super) fn create<const BUF_SIZE: usize>(
        mut tcp_reader: TcpReader<'_>,
        mut buffer: [u8; BUF_SIZE],
        pos: usize,
    ) -> TcpReaderReadFuture<'_, BUF_SIZE> {
        async move {
            let result = tcp_reader.read(&mut buffer[pos..]).await;
            (result, tcp_reader, buffer)
        }
    }
}

#[pin_project(
    project = TcpReaderOrReadFutureProj,
    project_replace = TcpReaderOrReadFutureProjOwn,
)]
pub(super) enum Status<'d, const BUF_SIZE: usize> {
    Data {
        tcp_reader: TcpReader<'d>,
        buffer: [u8; BUF_SIZE],
    },
    Future(#[pin] TcpReaderReadFuture<'d, BUF_SIZE>),
    Invalid,
}
