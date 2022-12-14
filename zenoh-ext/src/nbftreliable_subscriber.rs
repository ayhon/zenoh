//
// Copyright (c) 2022 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//
#[zenoh_core::unstable]
use {
    async_trait::async_trait,
    std::collections::hash_map::Entry,
    std::collections::HashMap,
    std::convert::TryFrom,
    std::future::Ready,
    std::sync::{Arc, Mutex},
    std::time::Duration,
    zenoh::handlers::{locked, DefaultHandler},
    zenoh::prelude::r#async::*,
    zenoh::query::{QueryTarget, Reply, ReplyKeyExpr},
    zenoh::sample::SourceInfo,
    zenoh::subscriber::{Reliability, Subscriber},
    zenoh::Result as ZResult,
    zenoh_collections::timer::Timer,
    zenoh_collections::{Timed, TimedEvent},
    zenoh_core::{zlock, AsyncResolve, Resolvable, SyncResolve},
};

/// The builder of NBFTReliableSubscriber, allowing to configure it.
#[zenoh_core::unstable]
pub struct NBFTReliableSubscriberBuilder<'b, Handler> {
    session: Arc<Session>,
    key_expr: ZResult<KeyExpr<'b>>,
    reliability: Reliability,
    origin: Locality,
    query_target: QueryTarget,
    query_timeout: Duration,
    period: Option<Duration>,
    history: bool,
    handler: Handler,
}

#[zenoh_core::unstable]
impl<'b> NBFTReliableSubscriberBuilder<'b, DefaultHandler> {
    pub(crate) fn new(
        session: Arc<Session>,
        key_expr: ZResult<KeyExpr<'b>>,
    ) -> NBFTReliableSubscriberBuilder<'b, DefaultHandler> {
        NBFTReliableSubscriberBuilder {
            session,
            key_expr,
            reliability: Reliability::default(),
            origin: Locality::default(),
            query_target: QueryTarget::BestMatching,
            query_timeout: Duration::from_secs(10),
            period: None,
            history: false,
            handler: DefaultHandler,
        }
    }

    /// Add callback to NBFTReliableSubscriber.
    #[inline]
    pub fn callback<Callback>(
        self,
        callback: Callback,
    ) -> NBFTReliableSubscriberBuilder<'b, Callback>
    where
        Callback: Fn(Sample) + Send + Sync + 'static,
    {
        let NBFTReliableSubscriberBuilder {
            session,
            key_expr,
            reliability,
            origin,
            query_target,
            query_timeout,
            period,
            history,
            handler: _,
        } = self;
        NBFTReliableSubscriberBuilder {
            session,
            key_expr,
            reliability,
            origin,
            query_target,
            query_timeout,
            period,
            history,
            handler: callback,
        }
    }

    /// Add callback to `NBFTReliableSubscriber`.
    ///
    /// Using this guarantees that your callback will never be called concurrently.
    /// If your callback is also accepted by the [`callback`](NBFTReliableSubscriberBuilder::callback) method, we suggest you use it instead of `callback_mut`
    #[inline]
    pub fn callback_mut<CallbackMut>(
        self,
        callback: CallbackMut,
    ) -> NBFTReliableSubscriberBuilder<'b, impl Fn(Sample) + Send + Sync + 'static>
    where
        CallbackMut: FnMut(Sample) + Send + Sync + 'static,
    {
        self.callback(locked(callback))
    }

    /// Make the built NBFTReliableSubscriber a [`NBFTReliableSubscriber`](NBFTReliableSubscriber).
    #[inline]
    pub fn with<Handler>(self, handler: Handler) -> NBFTReliableSubscriberBuilder<'b, Handler>
    where
        Handler: zenoh::prelude::IntoCallbackReceiverPair<'static, Sample>,
    {
        let NBFTReliableSubscriberBuilder {
            session,
            key_expr,
            reliability,
            origin,
            query_target,
            query_timeout,
            period,
            history,
            handler: _,
        } = self;
        NBFTReliableSubscriberBuilder {
            session,
            key_expr,
            reliability,
            origin,
            query_target,
            query_timeout,
            period,
            history,
            handler,
        }
    }
}

#[zenoh_core::unstable]
impl<'b, Handler> NBFTReliableSubscriberBuilder<'b, Handler> {
    /// Change the subscription reliability.
    #[inline]
    pub fn reliability(mut self, reliability: Reliability) -> Self {
        self.reliability = reliability;
        self
    }

    /// Change the subscription reliability to Reliable.
    #[inline]
    pub fn reliable(mut self) -> Self {
        self.reliability = Reliability::Reliable;
        self
    }

    /// Change the subscription reliability to BestEffort.
    #[inline]
    pub fn best_effort(mut self) -> Self {
        self.reliability = Reliability::BestEffort;
        self
    }

    /// Restrict the matching publications that will be receive by this [`Subscriber`]
    /// to the ones that have the given [`Locality`](crate::prelude::Locality).
    #[zenoh_core::unstable]
    #[inline]
    pub fn allowed_origin(mut self, origin: Locality) -> Self {
        self.origin = origin;
        self
    }

    /// Change the target to be used for queries.
    #[inline]
    pub fn query_target(mut self, query_target: QueryTarget) -> Self {
        self.query_target = query_target;
        self
    }

    /// Change the timeout to be used for queries.
    #[inline]
    pub fn query_timeout(mut self, query_timeout: Duration) -> Self {
        self.query_timeout = query_timeout;
        self
    }

    /// Enable periodic queries and specify queries period.
    #[inline]
    pub fn periodic_queries(mut self, period: Option<Duration>) -> Self {
        self.period = period;
        self
    }

    /// Enable/Disable query for historical data.
    #[inline]
    pub fn history(mut self, history: bool) -> Self {
        self.history = history;
        self
    }

    fn with_static_keys(self) -> NBFTReliableSubscriberBuilder<'static, Handler> {
        NBFTReliableSubscriberBuilder {
            session: self.session,
            key_expr: self.key_expr.map(|s| s.into_owned()),
            reliability: self.reliability,
            origin: self.origin,
            query_target: self.query_target,
            query_timeout: self.query_timeout,
            period: self.period,
            history: self.history,
            handler: self.handler,
        }
    }
}

#[zenoh_core::unstable]
impl<'a, Handler> Resolvable for NBFTReliableSubscriberBuilder<'a, Handler>
where
    Handler: IntoCallbackReceiverPair<'static, Sample>,
    Handler::Receiver: Send,
{
    type To = ZResult<NBFTReliableSubscriber<'a, Handler::Receiver>>;
}

#[zenoh_core::unstable]
impl<Handler> SyncResolve for NBFTReliableSubscriberBuilder<'_, Handler>
where
    Handler: IntoCallbackReceiverPair<'static, Sample> + Send,
    Handler::Receiver: Send,
{
    fn res_sync(self) -> <Self as Resolvable>::To {
        NBFTReliableSubscriber::new(self.with_static_keys())
    }
}

#[zenoh_core::unstable]
impl<Handler> AsyncResolve for NBFTReliableSubscriberBuilder<'_, Handler>
where
    Handler: IntoCallbackReceiverPair<'static, Sample> + Send,
    Handler::Receiver: Send,
{
    type Future = Ready<Self::To>;

    fn res_async(self) -> Self::Future {
        std::future::ready(self.res_sync())
    }
}

#[zenoh_core::unstable]
struct InnerState {
    last_seq_num: Option<ZInt>,
    pending_queries: u64,
    pending_samples: HashMap<ZInt, Sample>,
}

#[zenoh_core::unstable]
pub struct NBFTReliableSubscriber<'a, Receiver> {
    _subscriber: Subscriber<'a, ()>,
    receiver: Receiver,
}

#[zenoh_core::unstable]
impl<Receiver> std::ops::Deref for NBFTReliableSubscriber<'_, Receiver> {
    type Target = Receiver;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

#[zenoh_core::unstable]
impl<Receiver> std::ops::DerefMut for NBFTReliableSubscriber<'_, Receiver> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.receiver
    }
}

#[zenoh_core::unstable]
fn handle_sample(
    states: &mut HashMap<ZenohId, InnerState>,
    wait: bool,
    sample: Sample,
    callback: &Arc<dyn Fn(Sample) + Send + Sync>,
) -> bool {
    if let SourceInfo {
        source_id: Some(source_id),
        source_sn: Some(source_sn),
    } = sample.source_info
    {
        let entry = states.entry(source_id);
        let new = matches!(&entry, Entry::Occupied(_));
        let state = entry.or_insert(InnerState {
            last_seq_num: None,
            pending_queries: 0,
            pending_samples: HashMap::new(),
        });
        if wait {
            state.pending_samples.insert(source_sn, sample);
        } else if state.last_seq_num.is_some() && source_sn != state.last_seq_num.unwrap() + 1 {
            if source_sn > state.last_seq_num.unwrap() {
                state.pending_samples.insert(source_sn, sample);
            }
        } else {
            callback(sample);
            let mut last_seq_num = source_sn;
            state.last_seq_num = Some(last_seq_num);
            while let Some(s) = state.pending_samples.remove(&(last_seq_num + 1)) {
                callback(s);
                last_seq_num += 1;
                state.last_seq_num = Some(last_seq_num);
            }
        }
        new
    } else {
        callback(sample);
        true
    }
}

#[zenoh_core::unstable]
fn seq_num_range(start: Option<ZInt>, end: Option<ZInt>) -> String {
    match (start, end) {
        (Some(start), Some(end)) => format!("_sn={}..{}", start, end),
        (Some(start), None) => format!("_sn={}..", start),
        (None, Some(end)) => format!("_sn=..{}", end),
        (None, None) => "_sn=..".to_string(),
    }
}

#[zenoh_core::unstable]
#[derive(Clone)]
struct PeriodicQuery {
    source_id: ZenohId,
    statesref: Arc<Mutex<(HashMap<ZenohId, InnerState>, bool)>>,
    key_expr: KeyExpr<'static>,
    session: Arc<Session>,
    query_target: QueryTarget,
    query_timeout: Duration,
    callback: Arc<dyn Fn(Sample) + Send + Sync>,
}

#[zenoh_core::unstable]
impl PeriodicQuery {
    fn with_source_id(mut self, source_id: ZenohId) -> Self {
        self.source_id = source_id;
        self
    }
}

#[zenoh_core::unstable]
#[async_trait]
impl Timed for PeriodicQuery {
    async fn run(&mut self) {
        let mut lock = zlock!(self.statesref);
        let (states, _wait) = &mut *lock;
        if let Some(state) = states.get_mut(&self.source_id) {
            state.pending_queries += 1;
            let query_expr = (&self.source_id.into_keyexpr()) / &self.key_expr;
            let seq_num_range = seq_num_range(Some(state.last_seq_num.unwrap() + 1), None);
            drop(lock);
            let handler = RepliesHandler {
                source_id: self.source_id,
                statesref: self.statesref.clone(),
                callback: self.callback.clone(),
            };
            let _ = self
                .session
                .get(Selector::from(query_expr).with_parameters(&seq_num_range))
                .callback({
                    let key_expr = self.key_expr.clone().into_owned();
                    move |r: Reply| {
                        if let Ok(s) = r.sample {
                            if key_expr.intersects(&s.key_expr) {
                                let (ref mut states, wait) = &mut *zlock!(handler.statesref);
                                handle_sample(states, *wait, s, &handler.callback);
                            }
                        }
                    }
                })
                .consolidation(ConsolidationMode::None)
                .accept_replies(ReplyKeyExpr::Any)
                .target(self.query_target)
                .timeout(self.query_timeout)
                .res_sync();
        }
    }
}

#[zenoh_core::unstable]
impl<'a, Receiver> NBFTReliableSubscriber<'a, Receiver> {
    fn new<Handler>(conf: NBFTReliableSubscriberBuilder<'a, Handler>) -> ZResult<Self>
    where
        Handler: IntoCallbackReceiverPair<'static, Sample, Receiver = Receiver> + Send,
    {
        let statesref = Arc::new(Mutex::new((HashMap::new(), conf.history)));
        let (callback, receiver) = conf.handler.into_cb_receiver_pair();
        let key_expr = conf.key_expr?;
        let query_target = conf.query_target;
        let query_timeout = conf.query_timeout;
        let session = conf.session.clone();
        let periodic_query = conf.period.map(|period| {
            (
                Arc::new(Timer::new(false)),
                period,
                PeriodicQuery {
                    source_id: ZenohId::try_from([1]).unwrap(),
                    statesref: statesref.clone(),
                    key_expr: key_expr.clone().into_owned(),
                    session,
                    query_target,
                    query_timeout,
                    callback: callback.clone(),
                },
            )
        });

        let sub_callback = {
            let statesref = statesref.clone();
            let session = conf.session.clone();
            let callback = callback.clone();
            let key_expr = key_expr.clone().into_owned();
            let periodic_query = periodic_query.clone();

            move |s: Sample| {
                let mut lock = zlock!(statesref);
                let (states, wait) = &mut *lock;
                let source_id = s.source_info.source_id;
                let new = handle_sample(states, *wait, s, &callback);

                if let Some(source_id) = source_id {
                    if new {
                        if let Some((timer, period, query)) = periodic_query.as_ref() {
                            timer.add(TimedEvent::periodic(
                                *period,
                                query.clone().with_source_id(source_id),
                            ))
                        }
                    }

                    if let Some(state) = states.get_mut(&source_id) {
                        if state.pending_queries == 0 && !state.pending_samples.is_empty() {
                            state.pending_queries += 1;
                            let query_expr = (&source_id.into_keyexpr()) / &key_expr;
                            let seq_num_range =
                                seq_num_range(Some(state.last_seq_num.unwrap() + 1), None);
                            drop(lock);
                            let handler = RepliesHandler {
                                source_id,
                                statesref: statesref.clone(),
                                callback: callback.clone(),
                            };
                            let _ = session
                                .get(Selector::from(query_expr).with_parameters(&seq_num_range))
                                .callback({
                                    let key_expr = key_expr.clone().into_owned();
                                    move |r: Reply| {
                                        if let Ok(s) = r.sample {
                                            if key_expr.intersects(&s.key_expr) {
                                                let (ref mut states, wait) =
                                                    &mut *zlock!(handler.statesref);
                                                handle_sample(states, *wait, s, &handler.callback);
                                            }
                                        }
                                    }
                                })
                                .consolidation(ConsolidationMode::None)
                                .accept_replies(ReplyKeyExpr::Any)
                                .target(query_target)
                                .timeout(query_timeout)
                                .res_sync();
                        }
                    }
                }
            }
        };

        let subscriber = conf
            .session
            .declare_subscriber(&key_expr)
            .callback(sub_callback)
            .reliability(conf.reliability)
            .allowed_origin(conf.origin)
            .res_sync()?;

        if conf.history {
            let handler = InitialRepliesHandler {
                statesref,
                periodic_query,
                callback,
            };
            let _ = conf
                .session
                .get(
                    Selector::from(KeyExpr::try_from("*").unwrap() / &key_expr)
                        .with_parameters("0.."),
                )
                .callback({
                    let key_expr = key_expr.clone().into_owned();
                    move |r: Reply| {
                        if let Ok(s) = r.sample {
                            if key_expr.intersects(&s.key_expr) {
                                let (ref mut states, wait) = &mut *zlock!(handler.statesref);
                                handle_sample(states, *wait, s, &handler.callback);
                            }
                        }
                    }
                })
                .consolidation(ConsolidationMode::None)
                .accept_replies(ReplyKeyExpr::Any)
                .target(query_target)
                .timeout(query_timeout)
                .res_sync();
        }

        let reliable_subscriber = NBFTReliableSubscriber {
            _subscriber: subscriber,
            receiver,
        };

        Ok(reliable_subscriber)
    }

    /// Close this NBFTReliableSubscriber
    #[inline]
    pub fn close(self) -> impl Resolve<ZResult<()>> + 'a {
        self._subscriber.undeclare()
    }
}

#[zenoh_core::unstable]
#[derive(Clone)]
struct InitialRepliesHandler {
    statesref: Arc<Mutex<(HashMap<ZenohId, InnerState>, bool)>>,
    periodic_query: Option<(Arc<Timer>, Duration, PeriodicQuery)>,
    callback: Arc<dyn Fn(Sample) + Send + Sync>,
}

#[zenoh_core::unstable]
impl Drop for InitialRepliesHandler {
    fn drop(&mut self) {
        let (states, wait) = &mut *zlock!(self.statesref);
        for (source_id, state) in states.iter_mut() {
            let mut pending_samples = state
                .pending_samples
                .drain()
                .collect::<Vec<(ZInt, Sample)>>();
            pending_samples.sort_by_key(|(k, _s)| *k);
            for (seq_num, sample) in pending_samples {
                state.last_seq_num = Some(seq_num);
                (self.callback)(sample);
            }
            if let Some((timer, period, query)) = self.periodic_query.as_ref() {
                timer.add(TimedEvent::periodic(
                    *period,
                    query.clone().with_source_id(*source_id),
                ))
            }
        }
        *wait = false;
    }
}

#[zenoh_core::unstable]
#[derive(Clone)]
struct RepliesHandler {
    source_id: ZenohId,
    statesref: Arc<Mutex<(HashMap<ZenohId, InnerState>, bool)>>,
    callback: Arc<dyn Fn(Sample) + Send + Sync>,
}

#[zenoh_core::unstable]
impl Drop for RepliesHandler {
    fn drop(&mut self) {
        let (states, wait) = &mut *zlock!(self.statesref);
        if let Some(state) = states.get_mut(&self.source_id) {
            state.pending_queries -= 1;
            if !state.pending_samples.is_empty() && !*wait {
                log::error!("Sample missed: unable to retrieve some missing samples.");
                let mut pending_samples = state
                    .pending_samples
                    .drain()
                    .collect::<Vec<(ZInt, Sample)>>();
                pending_samples.sort_by_key(|(k, _s)| *k);
                for (seq_num, sample) in pending_samples {
                    state.last_seq_num = Some(seq_num);
                    (self.callback)(sample);
                }
            }
        }
    }
}
