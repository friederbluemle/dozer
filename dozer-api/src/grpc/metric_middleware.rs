use futures_util::future::BoxFuture;
use hyper::Body;
use metrics::{histogram, increment_counter};
use std::{
    task::{Context, Poll},
    time::Instant,
};
use tonic::{body::BoxBody, transport::NamedService};
use tower::{Layer, Service};

use crate::api_helper::{API_LATENCY_HISTOGRAM_NAME, API_REQUEST_COUNTER_NAME};

#[derive(Debug, Clone, Default)]
pub struct MetricMiddlewareLayer {}
impl MetricMiddlewareLayer {
    pub fn new() -> Self {
        Self {}
    }
}

impl<S> Layer<S> for MetricMiddlewareLayer {
    type Service = MetricMiddleware<S>;

    fn layer(&self, service: S) -> Self::Service {
        MetricMiddleware { inner: service }
    }
}

#[derive(Debug, Clone)]
pub struct MetricMiddleware<S> {
    inner: S,
}

impl<S> Service<hyper::Request<Body>> for MetricMiddleware<S>
where
    S: Service<hyper::Request<Body>, Response = hyper::Response<BoxBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: hyper::Request<Body>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        Box::pin(async move {
            let request_path = req
                .uri()
                .path_and_query()
                .map_or("".to_string(), |v| v.as_str().to_string());
            let start_time: Instant = Instant::now();
            let response = inner.call(req).await;
            if request_path.starts_with("/dozer.generated") {
                let cache_path = request_path
                    .split('/')
                    .nth(1)
                    .unwrap_or_default()
                    .split('.')
                    .nth(2)
                    .unwrap_or_default();
                let labels: [(&str, String); 2] = [
                    ("endpoint", cache_path.to_owned()),
                    ("api_type", "grpc".to_owned()),
                ];
                histogram!(API_LATENCY_HISTOGRAM_NAME, start_time.elapsed(), &labels);
                increment_counter!(API_REQUEST_COUNTER_NAME, &labels);
            }

            response
        })
    }
}

impl<S: NamedService> NamedService for MetricMiddleware<S> {
    const NAME: &'static str = S::NAME;
}
