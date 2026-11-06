use crate::{
    panics,
    server::{Client, LanguageServer, LspService},
    transport::{self, Ended},
};

/// Serves a language server over stdin and stdout until the client exits.
///
/// `build` makes the server from its [`Client`]. The loop is [`LspService`], which answers a
/// panicking request with an error and keeps the session going. Output goes through
/// [`transport`]'s queue, so a slow client does not slow the server.
///
/// `name` is the server's name in messages written on its behalf ("phrost failed while answering
/// ..."). The result says how the session ended, and [`Ended::exit_code`] is the exit status the
/// protocol requires.
pub async fn serve_stdio<S, F>(name: &'static str, build: F) -> Ended
where
    S: LanguageServer,
    F: FnOnce(Client) -> S,
{
    panics::install(name);
    let (service, socket) = LspService::new(build);
    transport::serve(
        name,
        tokio::io::stdin(),
        tokio::io::stdout(),
        socket,
        service,
    )
    .await
}
