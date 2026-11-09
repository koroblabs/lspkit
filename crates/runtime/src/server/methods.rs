//! The protocol methods a server can handle, as one trait, and the table that routes a message to
//! its method.
//!
//! The trait matches `tower-lsp-server`'s `LanguageServer` (MIT OR Apache-2.0) method for method,
//! in names, parameters and results. A request the server does not implement is answered with
//! `MethodNotFound`, and an unimplemented notification is dropped. The protocol specification
//! documents each method under its name.

use std::{future::Future, sync::Arc};

use futures_util::future::BoxFuture;
use ls_types::{
    request::{
        GotoDeclarationParams, GotoDeclarationResponse, GotoImplementationParams,
        GotoImplementationResponse, GotoTypeDefinitionParams, GotoTypeDefinitionResponse,
    },
    *,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::value::RawValue;

use crate::jsonrpc::{Error, Params, Result};

macro_rules! methods {
    (
        requests { $( $request:literal $answer:ident ( $asked:ty ) -> $answered:ty; )* }
        notifications { $( $notification:literal $handle:ident ( $told:ty ); )* }
    ) => {
        /// A language server. Implement the methods it handles; the defaults cover the rest.
        pub trait LanguageServer: Send + Sync + 'static {
            /// `initialize`: the first request, where the client states its capabilities.
            fn initialize(
                &self,
                params: InitializeParams,
            ) -> impl Future<Output = Result<InitializeResult>> + Send;

            /// `initialized`: the client has received the server's capabilities. Spawn anything
            /// slow, because notifications are handled one at a time, in order.
            fn initialized(&self, params: InitializedParams) -> impl Future<Output = ()> + Send {
                async move {
                    let _ = params;
                }
            }

            /// `shutdown`: answer, and expect `exit`.
            fn shutdown(&self) -> impl Future<Output = Result<()>> + Send;

            $(
                #[doc = concat!("`", $request, "`.")]
                fn $answer(&self, params: $asked) -> impl Future<Output = Result<$answered>> + Send {
                    async move {
                        let _ = params;
                        Err(Error::method_not_found())
                    }
                }
            )*

            $(
                #[doc = concat!("`", $notification, "`.")]
                fn $handle(&self, params: $told) -> impl Future<Output = ()> + Send {
                    async move {
                        let _ = params;
                    }
                }
            )*
        }

        /// `method` as the `'static` name the protocol defines, or `None` for a method outside the
        /// protocol. `jsonrpc::Message::parse` borrows a request's method through this instead of
        /// allocating a `String` for it on every message read.
        pub(crate) fn known(method: &str) -> Option<&'static str> {
            Some(match method {
                "initialize" => "initialize",
                "initialized" => "initialized",
                "shutdown" => "shutdown",
                "exit" => "exit",
                "$/cancelRequest" => "$/cancelRequest",
                "$/setTrace" => "$/setTrace",
                "$/logTrace" => "$/logTrace",
                "$/progress" => "$/progress",
                "window/workDoneProgress/cancel" => "window/workDoneProgress/cancel",
                $( $request => $request, )*
                $( $notification => $notification, )*
                _ => return None,
            })
        }

        /// The future answering request `method`, or `params` back when the protocol has no such
        /// request. Returning them lets a custom method take them without cloning every request's
        /// parameters up front.
        pub(crate) fn request<S: LanguageServer>(
            server: Arc<S>,
            method: &str,
            params: Option<Params>,
        ) -> std::result::Result<BoxFuture<'static, Result<Box<RawValue>>>, Option<Params>> {
            Ok(match method {
                "initialize" => answer(async move { server.initialize(parse(params)?).await }),
                "shutdown" => answer(async move { server.shutdown().await }),
                $( $request => answer(async move { server.$answer(parse(params)?).await }), )*
                _ => return Err(params),
            })
        }

        /// The future handling notification `method`, `Some(Err)` when its parameters do not
        /// parse, and `None` when the protocol has no such notification.
        pub(crate) fn notification<S: LanguageServer>(
            server: Arc<S>,
            method: &str,
            params: Option<Params>,
        ) -> Option<Result<BoxFuture<'static, ()>>> {
            Some(match method {
                "initialized" => parse(params).map(|params| -> BoxFuture<'static, ()> {
                    Box::pin(async move { server.initialized(params).await })
                }),
                $(
                    $notification => parse(params).map(|params| -> BoxFuture<'static, ()> {
                        Box::pin(async move { server.$handle(params).await })
                    }),
                )*
                _ => return None,
            })
        }
    };
}

/// Parses a handler's parameters from their JSON straight into its type. Missing `params` parse as
/// `null`, which a method without parameters accepts.
pub(crate) fn parse<T: DeserializeOwned>(params: Option<Params>) -> Result<T> {
    serde_json::from_str(params.as_ref().map_or("null", Params::get))
        .map_err(|err| Error::invalid_params(err.to_string()))
}

/// Serializes a handler's answer once, straight to the JSON the response carries. Going through a
/// `Value` cost four to five times as much on every response (see `jsonrpc::Response`).
pub(crate) fn answer<T: Serialize>(
    work: impl Future<Output = Result<T>> + Send + 'static,
) -> BoxFuture<'static, Result<Box<RawValue>>> {
    Box::pin(async move {
        let answered = work.await?;
        serde_json::value::to_raw_value(&answered).map_err(|_| Error::internal_error())
    })
}

methods! {
    requests {
        "textDocument/willSaveWaitUntil" will_save_wait_until(WillSaveTextDocumentParams) -> Option<Vec<TextEdit>>;
        "textDocument/declaration" goto_declaration(GotoDeclarationParams) -> Option<GotoDeclarationResponse>;
        "textDocument/definition" goto_definition(GotoDefinitionParams) -> Option<GotoDefinitionResponse>;
        "textDocument/typeDefinition" goto_type_definition(GotoTypeDefinitionParams) -> Option<GotoTypeDefinitionResponse>;
        "textDocument/implementation" goto_implementation(GotoImplementationParams) -> Option<GotoImplementationResponse>;
        "textDocument/references" references(ReferenceParams) -> Option<Vec<Location>>;
        "textDocument/prepareCallHierarchy" prepare_call_hierarchy(CallHierarchyPrepareParams) -> Option<Vec<CallHierarchyItem>>;
        "callHierarchy/incomingCalls" incoming_calls(CallHierarchyIncomingCallsParams) -> Option<Vec<CallHierarchyIncomingCall>>;
        "callHierarchy/outgoingCalls" outgoing_calls(CallHierarchyOutgoingCallsParams) -> Option<Vec<CallHierarchyOutgoingCall>>;
        "textDocument/prepareTypeHierarchy" prepare_type_hierarchy(TypeHierarchyPrepareParams) -> Option<Vec<TypeHierarchyItem>>;
        "typeHierarchy/supertypes" supertypes(TypeHierarchySupertypesParams) -> Option<Vec<TypeHierarchyItem>>;
        "typeHierarchy/subtypes" subtypes(TypeHierarchySubtypesParams) -> Option<Vec<TypeHierarchyItem>>;
        "textDocument/documentHighlight" document_highlight(DocumentHighlightParams) -> Option<Vec<DocumentHighlight>>;
        "textDocument/documentLink" document_link(DocumentLinkParams) -> Option<Vec<DocumentLink>>;
        "documentLink/resolve" document_link_resolve(DocumentLink) -> DocumentLink;
        "textDocument/hover" hover(HoverParams) -> Option<Hover>;
        "textDocument/codeLens" code_lens(CodeLensParams) -> Option<Vec<CodeLens>>;
        "codeLens/resolve" code_lens_resolve(CodeLens) -> CodeLens;
        "textDocument/foldingRange" folding_range(FoldingRangeParams) -> Option<Vec<FoldingRange>>;
        "textDocument/selectionRange" selection_range(SelectionRangeParams) -> Option<Vec<SelectionRange>>;
        "textDocument/documentSymbol" document_symbol(DocumentSymbolParams) -> Option<DocumentSymbolResponse>;
        "textDocument/semanticTokens/full" semantic_tokens_full(SemanticTokensParams) -> Option<SemanticTokensResult>;
        "textDocument/semanticTokens/full/delta" semantic_tokens_full_delta(SemanticTokensDeltaParams) -> Option<SemanticTokensFullDeltaResult>;
        "textDocument/semanticTokens/range" semantic_tokens_range(SemanticTokensRangeParams) -> Option<SemanticTokensRangeResult>;
        "textDocument/inlineValue" inline_value(InlineValueParams) -> Option<Vec<InlineValue>>;
        "textDocument/inlayHint" inlay_hint(InlayHintParams) -> Option<Vec<InlayHint>>;
        "inlayHint/resolve" inlay_hint_resolve(InlayHint) -> InlayHint;
        "textDocument/moniker" moniker(MonikerParams) -> Option<Vec<Moniker>>;
        "textDocument/completion" completion(CompletionParams) -> Option<CompletionResponse>;
        "completionItem/resolve" completion_resolve(CompletionItem) -> CompletionItem;
        "textDocument/diagnostic" diagnostic(DocumentDiagnosticParams) -> DocumentDiagnosticReportResult;
        "workspace/diagnostic" workspace_diagnostic(WorkspaceDiagnosticParams) -> WorkspaceDiagnosticReportResult;
        "textDocument/signatureHelp" signature_help(SignatureHelpParams) -> Option<SignatureHelp>;
        "textDocument/codeAction" code_action(CodeActionParams) -> Option<CodeActionResponse>;
        "codeAction/resolve" code_action_resolve(CodeAction) -> CodeAction;
        "textDocument/documentColor" document_color(DocumentColorParams) -> Vec<ColorInformation>;
        "textDocument/colorPresentation" color_presentation(ColorPresentationParams) -> Vec<ColorPresentation>;
        "textDocument/formatting" formatting(DocumentFormattingParams) -> Option<Vec<TextEdit>>;
        "textDocument/rangeFormatting" range_formatting(DocumentRangeFormattingParams) -> Option<Vec<TextEdit>>;
        "textDocument/onTypeFormatting" on_type_formatting(DocumentOnTypeFormattingParams) -> Option<Vec<TextEdit>>;
        "textDocument/rename" rename(RenameParams) -> Option<WorkspaceEdit>;
        "textDocument/prepareRename" prepare_rename(TextDocumentPositionParams) -> Option<PrepareRenameResponse>;
        "textDocument/linkedEditingRange" linked_editing_range(LinkedEditingRangeParams) -> Option<LinkedEditingRanges>;
        "workspace/symbol" symbol(WorkspaceSymbolParams) -> Option<WorkspaceSymbolResponse>;
        "workspaceSymbol/resolve" symbol_resolve(WorkspaceSymbol) -> WorkspaceSymbol;
        "workspace/willCreateFiles" will_create_files(CreateFilesParams) -> Option<WorkspaceEdit>;
        "workspace/willRenameFiles" will_rename_files(RenameFilesParams) -> Option<WorkspaceEdit>;
        "workspace/willDeleteFiles" will_delete_files(DeleteFilesParams) -> Option<WorkspaceEdit>;
        "workspace/executeCommand" execute_command(ExecuteCommandParams) -> Option<LSPAny>;
    }
    notifications {
        "textDocument/didOpen" did_open(DidOpenTextDocumentParams);
        "textDocument/didChange" did_change(DidChangeTextDocumentParams);
        "textDocument/willSave" will_save(WillSaveTextDocumentParams);
        "textDocument/didSave" did_save(DidSaveTextDocumentParams);
        "textDocument/didClose" did_close(DidCloseTextDocumentParams);
        "notebookDocument/didOpen" notebook_did_open(DidOpenNotebookDocumentParams);
        "notebookDocument/didChange" notebook_did_change(DidChangeNotebookDocumentParams);
        "notebookDocument/didSave" notebook_did_save(DidSaveNotebookDocumentParams);
        "notebookDocument/didClose" notebook_did_close(DidCloseNotebookDocumentParams);
        "workspace/didChangeConfiguration" did_change_configuration(DidChangeConfigurationParams);
        "workspace/didChangeWorkspaceFolders" did_change_workspace_folders(DidChangeWorkspaceFoldersParams);
        "workspace/didCreateFiles" did_create_files(CreateFilesParams);
        "workspace/didRenameFiles" did_rename_files(RenameFilesParams);
        "workspace/didDeleteFiles" did_delete_files(DeleteFilesParams);
        "workspace/didChangeWatchedFiles" did_change_watched_files(DidChangeWatchedFilesParams);
    }
}
