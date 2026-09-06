import { EmojiPicker } from './EmojiPicker';
import { GifPicker, isGifPickerAvailable } from './GifPicker';
import { VoiceRecorder } from './VoiceRecorder';
import { useMessageComposer } from './composer/useMessageComposer';

export function MessageInput() {
  const {
    commandError,
    setCommandError,
    invokingCommand,
    activeServer,
    activeChannel,
    activeDirectConversation,
    directConversation,
    key,
    compositionKey,
    setUploadState,
    uploading,
    uploadProgress,
    uploadError,
    voiceRetryBlob,
    pendingFiles,
    failedCompositions,
    retryFailedComposition,
    dismissFailedComposition,
    text,
    replyingTo,
    setReplyFor,
    composingRef,
    inputRef,
    fileInputRef,
    showGifPicker,
    setShowGifPicker,
    isRecording,
    setIsRecording,
    showEmojiPicker,
    setShowEmojiPicker,
    slashIndex,
    slashQuery,
    slashCandidates,
    insertSlashCommand,
    mentionQuery,
    mentionIndex,
    mentionCandidates,
    insertMention,
    handleVoiceRecorded,
    handleEmojiSelect,
    handleGifSelect,
    handleFileSelect,
    removePendingFile,
    handleSend,
    cancelUploads,
    handleKeyDown,
    handleChange,
  } = useMessageComposer();
  if (!activeChannel && !activeDirectConversation) return null;

  if (isRecording) {
    return (
      <div className="px-4 pb-6 pt-1">
        <VoiceRecorder
          onRecorded={handleVoiceRecorded}
          onCancel={() => setIsRecording(false)}
        />
      </div>
    );
  }

  return (
    <div className="px-4 pb-6 pt-1">
      {uploadError && (
        <div role="alert" className="mb-2 flex items-center gap-2 rounded border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-text-secondary">
          <span className="min-w-0 flex-1">{uploadError}</span>
          <button className="font-medium text-accent" onClick={() => {
            if (voiceRetryBlob) void handleVoiceRecorded(voiceRetryBlob);
            else void handleSend();
          }}>Retry</button>
          <button className="text-text-muted" aria-label="Dismiss upload error" onClick={() => {
            if (compositionKey) setUploadState(compositionKey, { error: null, voiceRetryBlob: null });
          }}>×</button>
        </div>
      )}
      {commandError && (
        <div role="alert" className="mb-2 flex items-center gap-2 rounded border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-text-secondary">
          <span className="min-w-0 flex-1">{commandError}</span>
          <button className="text-text-muted" aria-label="Dismiss command error" onClick={() => setCommandError(null)}>×</button>
        </div>
      )}
      {uploading && (
        <div className="mb-2 flex items-center gap-2 text-xs text-text-secondary" role="status" aria-live="polite">
          <progress className="h-2 flex-1" max={1} value={uploadProgress}>Uploading {Math.round(uploadProgress * 100)}%</progress>
          <span>{Math.round(uploadProgress * 100)}%</span>
          <button className="font-medium text-red-400" onClick={cancelUploads}>Cancel upload</button>
        </div>
      )}
      {failedCompositions.map((failed) => (
        <div key={failed.id} role="alert" className="mb-2 flex items-center gap-2 rounded border border-red-500/40 bg-red-500/10 px-3 py-2 text-xs text-text-secondary">
          <span className="min-w-0 flex-1 truncate">Failed: {failed.content} ({failed.error})</span>
          <button className="font-medium text-accent" onClick={() => retryFailedComposition(failed.id)}>Retry</button>
          <button className="text-text-muted" aria-label="Dismiss failed message" onClick={() => dismissFailedComposition(failed.id)}>×</button>
        </div>
      ))}
      {/* Reply bar */}
      {replyingTo && (
        <div className="mb-1 flex items-center gap-2 rounded-t-lg bg-bg-secondary px-4 py-2 text-sm">
          <svg className="h-4 w-4 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M3 10h10a5 5 0 015 5v3M3 10l4-4M3 10l4 4" />
          </svg>
          <span className="text-text-muted">Replying to</span>
          <span className="font-medium text-text-primary">{replyingTo.from}</span>
          <span className="min-w-0 flex-1 truncate text-text-muted">{replyingTo.content_preview}</span>
          <button
            onClick={() => { if (key) setReplyFor(key, null); }}
            className="shrink-0 text-text-muted hover:text-text-primary"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      )}
      {/* Pending file previews */}
      {pendingFiles.length > 0 && (
        <div className={`flex flex-wrap gap-2 bg-bg-input px-4 pt-3 ${replyingTo ? '' : 'rounded-t-lg'}`}>
          {pendingFiles.map((file, i) => (
            <div key={`${file.name}-${i}`} className="relative flex items-center gap-2 rounded bg-bg-secondary px-3 py-2 text-sm">
              {file.type.startsWith('image/') ? (
                <img
                  src={URL.createObjectURL(file)}
                  alt={file.name}
                  className="h-10 w-10 rounded object-cover"
                />
              ) : (
                <svg className="h-5 w-5 shrink-0 text-text-muted" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
                </svg>
              )}
              <span className="max-w-[120px] truncate text-text-secondary">{file.name}</span>
              <button
                onClick={() => removePendingFile(i)}
                aria-label={`Remove ${file.name}`}
                className="ml-1 shrink-0 text-text-muted hover:text-red-400"
              >
                <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          ))}
        </div>
      )}
      <div className={`relative flex items-center bg-bg-input px-4 ${replyingTo && pendingFiles.length === 0 ? 'rounded-b-lg' :
          pendingFiles.length > 0 ? 'rounded-b-lg' :
            'rounded-lg'
        }`}>
        {slashQuery !== null && slashCandidates.length > 0 && (
          <div
            role="listbox"
            aria-label="Slash commands"
            className="absolute bottom-full left-0 right-0 mb-1 max-h-56 overflow-y-auto rounded-lg border border-border bg-bg-secondary shadow-lg"
          >
            {slashCandidates.map((command, index) => (
              <button
                key={command.id}
                type="button"
                role="option"
                aria-selected={index === slashIndex}
                onMouseDown={(event) => { event.preventDefault(); insertSlashCommand(command.name); }}
                className={`flex w-full flex-col px-3 py-2 text-left text-sm ${index === slashIndex ? 'bg-bg-active' : 'hover:bg-bg-hover'}`}
              >
                <span className="font-medium text-blue-300">/{command.name}</span>
                <span className="text-xs text-text-muted">{command.description}</span>
              </button>
            ))}
          </div>
        )}
        {/* Mention autocomplete popup */}
        {mentionQuery !== null && mentionCandidates.length > 0 && (
          <div className="absolute bottom-full left-0 right-0 mb-1 max-h-48 overflow-y-auto rounded-lg border border-border bg-bg-secondary shadow-lg">
            {mentionCandidates.map((candidate, i) => (
              <button
                key={candidate}
                onMouseDown={(e) => { e.preventDefault(); insertMention(candidate); }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm ${i === mentionIndex
                    ? 'bg-bg-active text-text-primary'
                    : 'text-text-secondary hover:bg-bg-hover'
                  }`}
              >
                <span className="font-medium text-blue-300">{candidate}</span>
              </button>
            ))}
          </div>
        )}
        {/* GIF picker */}
        {showGifPicker && (
          <GifPicker onSelect={handleGifSelect} onClose={() => setShowGifPicker(false)} />
        )}
        <input type="file" ref={fileInputRef} onChange={handleFileSelect} className="hidden" multiple />
        <button
          onClick={() => fileInputRef.current?.click()}
          className="mr-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary"
          title="Upload file"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M12 4v16m8-8H4" />
          </svg>
        </button>
        <button
          onClick={() => setShowGifPicker((v) => !v)}
          disabled={!isGifPickerAvailable()}
          className="mr-2 hidden rounded p-1.5 text-text-muted transition-colors hover:text-text-primary disabled:cursor-not-allowed disabled:opacity-40 sm:block"
          title={isGifPickerAvailable() ? 'GIF picker' : 'GIF picker unavailable: no provider is configured'}
          aria-label={isGifPickerAvailable() ? 'Open GIF picker' : 'GIF picker unavailable: no provider is configured'}
        >
          <span className="text-xs font-bold">GIF</span>
        </button>
        <button
          onClick={() => setShowEmojiPicker((v) => !v)}
          className="mr-2 hidden rounded p-1.5 text-text-muted transition-colors hover:text-text-primary sm:block"
          title="Emoji picker"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M14.828 14.828a4 4 0 01-5.656 0M9 10h.01M15 10h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
        </button>
        {/* Emoji picker */}
        {showEmojiPicker && (
          <EmojiPicker onSelect={handleEmojiSelect} onClose={() => setShowEmojiPicker(false)} serverId={activeServer} />
        )}
        <textarea
          ref={inputRef}
          rows={1}
          value={text}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          onCompositionStart={() => { composingRef.current = true; }}
          onCompositionEnd={() => { composingRef.current = false; }}
          placeholder={`Message ${directConversation ? `@${directConversation.peer_username}` : activeChannel}`}
          aria-autocomplete={slashQuery !== null || mentionQuery !== null ? 'list' : 'none'}
          className="max-h-40 min-h-6 flex-1 resize-none overflow-y-auto bg-transparent py-3 text-text-primary placeholder-text-muted outline-none"
        />
        <button
          onClick={() => setIsRecording(true)}
          className="ml-2 hidden rounded p-1.5 text-text-muted transition-colors hover:text-text-primary sm:block"
          title="Record voice message"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4M12 15a3 3 0 003-3V5a3 3 0 00-6 0v7a3 3 0 003 3z" />
          </svg>
        </button>
        <button
          onClick={handleSend}
          aria-label="Send message"
          disabled={(!text.trim() && pendingFiles.length === 0) || uploading || invokingCommand}
          className="ml-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary disabled:opacity-30"
        >
          {uploading || invokingCommand ? (
            <svg className="h-5 w-5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
          ) : (
            <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
              <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
            </svg>
          )}
        </button>
      </div>
    </div>
  );
}
