use super::{OnEos, OnFailure};
use crate::classify::ClassifyEos;
use http::HeaderMap;
use http_body::Body;
use pin_project::pin_project;
use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};
use tracing::Span;

/// Response body for [`Trace`].
///
/// [`Trace`]: super::Trace
#[pin_project]
pub struct ResponseBody<B, C, OnEos, OnFailure> {
    #[pin]
    pub(crate) inner: B,
    pub(crate) classify_eos: Option<C>,
    pub(crate) on_eos: Option<(OnEos, Instant)>,
    pub(crate) on_failure: Option<OnFailure>,
    pub(crate) start: Instant,
    pub(crate) span: Span,
}

impl<B, C, OnEosT, OnFailureT> Body for ResponseBody<B, C, OnEosT, OnFailureT>
where
    B: Body,
    C: ClassifyEos<B::Error>,
    OnEosT: OnEos,
    OnFailureT: OnFailure<C::FailureClass>,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        let this = self.project();
        let _guard = this.span.enter();
        let latency = this.start.elapsed();

        if let Some(result) = futures_util::ready!(this.inner.poll_data(cx)) {
            if let Some((classify_eos, on_failure)) =
                this.classify_eos.take().zip(this.on_failure.take())
            {
                match &result {
                    Ok(_chunk) => {}
                    Err(err) => {
                        let failure_class = classify_eos.classify_error(err);
                        on_failure.on_failure(failure_class, latency);
                    }
                }
            }

            Poll::Ready(Some(result))
        } else {
            Poll::Ready(None)
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        let this = self.project();
        let _guard = this.span.enter();
        let result = futures_util::ready!(this.inner.poll_trailers(cx));

        let latency = this.start.elapsed();

        if let Some((classify_eos, on_failure)) =
            this.classify_eos.take().zip(this.on_failure.take())
        {
            match &result {
                Ok(trailers) => {
                    if let Err(failure_class) = classify_eos.classify_eos(trailers.as_ref()) {
                        on_failure.on_failure(failure_class, latency);
                    } else if let Some((on_eos, stream_start)) = this.on_eos.take() {
                        on_eos.on_eos(trailers.as_ref(), stream_start.elapsed());
                    }
                }
                Err(err) => {
                    let failure_class = classify_eos.classify_error(err);
                    on_failure.on_failure(failure_class, latency);
                }
            }
        }

        Poll::Ready(result)
    }
}