//! Work-done progress bars, created the way the protocol requires. A client that ignores the
//! request to create one is not asked again in the session.

use std::sync::atomic::{AtomicBool, Ordering};

use ls_types::{
    ClientCapabilities, ProgressParams, ProgressParamsValue, ProgressToken, WorkDoneProgress,
    WorkDoneProgressBegin, WorkDoneProgressEnd, WorkDoneProgressReport,
    notification::Progress as Reported,
};

use crate::{client, server::Client};

/// Whether the client supports progress bars, and whether it refused or ignored a request for one.
#[derive(Debug, Default)]
pub struct Progress {
    supported: AtomicBool,
    refused: AtomicBool,
}

impl Progress {
    pub const fn new() -> Progress {
        Progress {
            supported: AtomicBool::new(false),
            refused: AtomicBool::new(false),
        }
    }

    /// Reads `window.workDoneProgress` from the client's capabilities, at `initialize`.
    pub fn negotiate(&self, capabilities: &ClientCapabilities) {
        let supported = capabilities
            .window
            .as_ref()
            .and_then(|window| window.work_done_progress)
            .unwrap_or(false);
        self.supported.store(supported, Ordering::Relaxed);
    }

    /// Starts a bar titled `title` over `total` steps of `unit` ("files"). The [`Bar`] draws
    /// nothing when the client does not support bars.
    ///
    /// The token is created before any progress is reported, as the protocol requires. Zed
    /// silently drops `$/progress` for a token it was not asked to create, so without this there
    /// is no bar and the server looks hung. Creating it costs one round trip, bounded by
    /// [`client::REPLY_TIMEOUT`]. A client that ignores the request is not asked again this
    /// session, so the wait happens once.
    pub async fn start(
        &self,
        client: &Client,
        token: &str,
        title: &str,
        total: usize,
        unit: &'static str,
    ) -> Bar {
        let mut bar = Bar {
            shown: None,
            total,
            unit,
            reported: 0,
        };
        if !self.supported.load(Ordering::Relaxed) || self.refused.load(Ordering::Relaxed) {
            return bar;
        }
        let token = ProgressToken::String(token.to_owned());
        let granted = client::ask(
            client,
            &format!("show a progress bar for {title}"),
            client.create_work_done_progress(token.clone()),
        )
        .await;
        self.refused.store(!granted, Ordering::Relaxed);
        if granted {
            let begin = WorkDoneProgress::Begin(WorkDoneProgressBegin {
                title: title.to_owned(),
                cancellable: Some(false),
                message: Some(format!("{total} {unit}")),
                percentage: Some(0),
            });
            send(client, &token, begin).await;
            bar.shown = Some((client.clone(), token));
        }
        bar
    }
}

async fn send(client: &Client, token: &ProgressToken, value: WorkDoneProgress) {
    client
        .send_notification::<Reported>(ProgressParams {
            token: token.clone(),
            value: ProgressParamsValue::WorkDone(value),
        })
        .await;
}

/// One operation's progress bar.
#[derive(Debug)]
#[must_use = "a bar the client is drawing stays on its screen until it is finished"]
pub struct Bar {
    /// The client and the token, or `None` when no bar is shown.
    shown: Option<(Client, ProgressToken)>,
    total: usize,
    unit: &'static str,
    /// The last percentage sent.
    reported: u32,
}

impl Bar {
    /// Reports that `done` steps are complete. A notification is sent only when the whole-number
    /// percentage rises, so a bar sends at most a hundred.
    pub async fn reached(&mut self, done: usize) {
        let Some((client, token)) = &self.shown else {
            return;
        };
        let percentage =
            u32::try_from(done.min(self.total) * 100 / self.total.max(1)).unwrap_or(100);
        if percentage <= self.reported {
            return;
        }
        self.reported = percentage;
        let report = WorkDoneProgress::Report(WorkDoneProgressReport {
            cancellable: None,
            message: Some(format!("{done} of {} {}", self.total, self.unit)),
            percentage: Some(percentage),
        });
        send(client, token, report).await;
    }

    /// Ends the bar with a final `message`.
    pub async fn finish(self, message: impl Into<String>) {
        if let Some((client, token)) = &self.shown {
            let end = WorkDoneProgress::End(WorkDoneProgressEnd {
                message: Some(message.into()),
            });
            send(client, token, end).await;
        }
    }

    /// Whether the client is drawing this bar.
    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }
}
