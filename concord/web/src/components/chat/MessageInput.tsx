import { useRef, useState, useCallback, useEffect, useMemo, type KeyboardEvent, type SetStateAction } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { useComposerStore } from '../../stores/composerStore';
import { channelKey } from '../../api/types';
import { uploadFile } from '../../api/client';
import type { MemberInfo, SlashCommandInfo } from '../../api/types';
import { GifPicker, isGifPickerAvailable } from './GifPicker';
import { VoiceRecorder } from './VoiceRecorder';
import { EmojiPicker } from './EmojiPicker';

const EMPTY_MEMBERS: MemberInfo[] = [];
const EMPTY_FILES: File[] = [];
const EMPTY_SLASH_COMMANDS: SlashCommandInfo[] = [];
const EMPTY_UPLOAD = { uploading: false, progress: 0, error: null, voiceRetryBlob: null, controllers: [] } as const;

export function MessageInput() {
  const [commandError, setCommandError] = useState<string | null>(null);
  const [invokingCommand, setInvokingCommand] = useState(false);
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const activeDirectConversation = useUiStore((s) => s.activeDirectConversation);
  const directConversation = useChatStore((s) => s.directConversations.find((dm) => dm.id === activeDirectConversation));
  const activeAccountId = useChatStore((s) => s.activeAccountId);
  const accountGeneration = useChatStore((s) => s.accountGeneration);
  const connected = useChatStore((s) => s.connected);
  const key = activeDirectConversation ? `dm:${activeDirectConversation}` : activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
  const compositionKey = activeAccountId && key ? `${activeAccountId}:${key}` : null;
  const uploadState = useComposerStore((s) => compositionKey ? s.uploads[compositionKey] ?? EMPTY_UPLOAD : EMPTY_UPLOAD);
  const setUploadState = useComposerStore((s) => s.setUploadState);
  const clearUploadState = useComposerStore((s) => s.clearUploadState);
  const { uploading, progress: uploadProgress, error: uploadError, voiceRetryBlob } = uploadState;
  const pendingFiles = useComposerStore((s) => compositionKey
    ? s.compositionFiles[compositionKey] ?? EMPTY_FILES
    : EMPTY_FILES);
  const setCompositionFiles = useComposerStore((s) => s.setCompositionFiles);
  const allFailedCompositions = useComposerStore((s) => s.failedCompositions);
  const failedCompositions = useMemo(() => allFailedCompositions.filter((failed) =>
    failed.accountId === activeAccountId && failed.key === key), [allFailedCompositions, activeAccountId, key]);
  const retryFailedComposition = useChatStore((s) => s.retryFailedComposition);
  const dismissFailedComposition = useChatStore((s) => s.dismissFailedComposition);
  const text = useComposerStore((s) => (key ? s.drafts[key] ?? '' : ''));
  const setDraft = useComposerStore((s) => s.setDraft);
  const setText = useCallback((next: SetStateAction<string>) => {
    if (!key) return;
    setDraft(key, typeof next === 'function' ? next(text) : next);
  }, [key, setDraft, text]);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const sendDirectMessage = useChatStore((s) => s.sendDirectMessage);
  const sendTyping = useChatStore((s) => s.sendTyping);
  const listSlashCommands = useChatStore((s) => s.listSlashCommands);
  const invokeSlashCommand = useChatStore((s) => s.invokeSlashCommand);
  const slashCommands = useChatStore((s) => activeServer
    ? s.slashCommands[activeServer] ?? EMPTY_SLASH_COMMANDS
    : EMPTY_SLASH_COMMANDS);
  const replyingTo = useComposerStore((s) => key ? s.replies[key] ?? null : null);
  const setReplyFor = useComposerStore((s) => s.setReplyFor);
  const lastTypingRef = useRef(0);
  const composingRef = useRef(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [showGifPicker, setShowGifPicker] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [showEmojiPicker, setShowEmojiPicker] = useState(false);
  const [slashIndex, setSlashIndex] = useState(0);

  useEffect(() => {
    if (connected && activeServer && !activeDirectConversation) listSlashCommands(activeServer);
  }, [connected, activeServer, activeDirectConversation, listSlashCommands]);

  const slashQuery = !activeDirectConversation && text.startsWith('/') && !text.slice(1).includes(' ')
    ? text.slice(1).toLowerCase()
    : null;
  const slashCandidates = slashQuery === null
    ? []
    : slashCommands.filter((command) => command.name.toLowerCase().startsWith(slashQuery)).slice(0, 8);

  useEffect(() => setSlashIndex(0), [slashQuery, slashCandidates.length]);

  const insertSlashCommand = useCallback((name: string) => {
    setText(`/${name} `);
    setTimeout(() => inputRef.current?.focus(), 0);
  }, [setText]);

  const updatePendingFiles = useCallback((next: SetStateAction<File[]>) => {
    if (!compositionKey) return;
    const current = useComposerStore.getState().compositionFiles[compositionKey] ?? EMPTY_FILES;
    setCompositionFiles(compositionKey, typeof next === 'function' ? next(current) : next);
  }, [compositionKey, setCompositionFiles]);

  // Mention autocomplete state
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionStart, setMentionStart] = useState(0); // cursor position of the '@'
  const members = useChatStore((s) => (key ? s.members[key] ?? EMPTY_MEMBERS : EMPTY_MEMBERS));

  const mentionCandidates = mentionQuery !== null
    ? [
        ...(['everyone', 'here'].filter((g) => g.startsWith(mentionQuery.toLowerCase())).map((g) => `@${g}`)),
        ...members
          .filter((m) => m.nickname.toLowerCase().startsWith(mentionQuery.toLowerCase()))
          .map((m) => `@${m.nickname}`),
      ].slice(0, 8)
    : [];

  // Reset mention index when candidates change
  useEffect(() => {
    setMentionIndex(0);
  }, [mentionCandidates.length]);

  const insertMention = useCallback((mention: string) => {
    const before = text.slice(0, mentionStart);
    const after = text.slice(mentionStart + (mentionQuery?.length ?? 0) + 1);
    setText(before + mention + ' ' + after);
    setMentionQuery(null);
    // Focus back on input
    setTimeout(() => inputRef.current?.focus(), 0);
  }, [text, mentionStart, mentionQuery, setText]);

  const handleVoiceRecorded = useCallback(async (blob: Blob) => {
    if (((!activeServer || !activeChannel) && !activeDirectConversation) || !compositionKey) return;
    const capturedGeneration = accountGeneration;
    const capturedAccount = activeAccountId;
    setIsRecording(false);
    const controller = new AbortController();
    setUploadState(compositionKey, { uploading: true, error: null, voiceRetryBlob: null, progress: 0, controllers: [controller] });
    try {
      const file = new File([blob], `voice-message-${Date.now()}.webm`, { type: blob.type });
      const uploaded = await uploadFile(file, activeDirectConversation
        ? { conversationId: activeDirectConversation }
        : { serverId: activeServer!, channel: activeChannel! }, {
          signal: controller.signal,
          onProgress: (loaded, total) => setUploadState(compositionKey, { progress: total > 0 ? loaded / total : 0 }),
        });
      const current = useChatStore.getState();
      const ui = useUiStore.getState();
      if (current.accountGeneration !== capturedGeneration
          || current.activeAccountId !== capturedAccount
          || ui.activeServer !== activeServer
          || ui.activeChannel !== activeChannel
          || ui.activeDirectConversation !== activeDirectConversation) {
        setUploadState(compositionKey, { voiceRetryBlob: blob, error: 'Conversation changed before the voice message could be sent.' });
        return;
      }
      const accepted = activeDirectConversation && directConversation
        ? sendDirectMessage(activeDirectConversation, directConversation.peer_username, '', [uploaded])
        : sendMessage(activeServer!, activeChannel!, '', [uploaded]);
      if (!accepted) {
        setUploadState(compositionKey, { voiceRetryBlob: blob, error: 'Voice message was uploaded but could not be sent.' });
      }
    } catch (err) {
      if (!(err instanceof DOMException && err.name === 'AbortError')) {
        setUploadState(compositionKey, { voiceRetryBlob: blob, error: err instanceof Error ? err.message : 'Voice upload failed' });
      }
    } finally {
      setUploadState(compositionKey, { controllers: [], uploading: false });
    }
  }, [accountGeneration, activeAccountId, activeServer, activeChannel, activeDirectConversation, compositionKey, directConversation, sendMessage, sendDirectMessage, setUploadState]);

  const handleEmojiSelect = (emoji: string) => {
    setText((prev) => prev + emoji);
    inputRef.current?.focus();
  };

  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.style.height = 'auto';
    input.style.height = `${Math.min(input.scrollHeight, 160)}px`;
  }, [text]);

  const handleGifSelect = (url: string) => {
    if (activeDirectConversation && directConversation) {
      if (!sendDirectMessage(activeDirectConversation, directConversation.peer_username, url)) setText(url);
    } else if (activeServer && activeChannel) {
      if (!sendMessage(activeServer, activeChannel, url)) setText(url);
    }
    setShowGifPicker(false);
  };

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files) {
      updatePendingFiles((prev) => [...prev, ...Array.from(e.target.files!)]);
    }
    // Reset so the same file can be re-selected
    e.target.value = '';
  };

  const removePendingFile = (index: number) => {
    updatePendingFiles((prev) => prev.filter((_, i) => i !== index));
  };

  const handleSend = async () => {
    const trimmed = text.trim();
    if ((!trimmed && pendingFiles.length === 0)
        || ((!activeChannel || !activeServer) && (!activeDirectConversation || !directConversation))) return;
    if (!activeDirectConversation && activeServer && activeChannel && trimmed.startsWith('/')) {
      const [rawName, ...rawArgs] = trimmed.slice(1).split(/\s+/);
      const command = slashCommands.find((candidate) => candidate.name.toLowerCase() === rawName.toLowerCase());
      if (command) {
        const args = Object.fromEntries(command.options.map((option, index) => {
          const raw = rawArgs[index];
          if (option.option_type === 'integer') return [option.name, raw === undefined ? null : Number.parseInt(raw, 10)];
          if (option.option_type === 'boolean') return [option.name, raw === undefined ? null : raw.toLowerCase() === 'true'];
          return [option.name, raw ?? null];
        }).filter(([, value]) => value !== null));
        setInvokingCommand(true);
        setCommandError(null);
        try {
          await invokeSlashCommand(activeServer, activeChannel, command.name, JSON.stringify(args));
          if (key && useComposerStore.getState().drafts[key] === text) setDraft(key, '');
        } catch (error) {
          setCommandError(error instanceof Error ? error.message : 'Command was not accepted.');
        } finally {
          setInvokingCommand(false);
        }
        return;
      }
    }
    const capturedGeneration = accountGeneration;
    const capturedAccount = activeAccountId;
    const capturedServer = activeServer;
    const capturedChannel = activeChannel;
    const capturedDirectConversation = activeDirectConversation;
    const capturedComposition = compositionKey;
    const capturedFiles = [...pendingFiles];

    let attachments: import('../../api/types').AttachmentInfo[] | undefined;

    if (pendingFiles.length > 0) {
      if (!capturedComposition) return;
      setUploadState(capturedComposition, { uploading: true, error: null, progress: 0 });
      const progress = new Map<number, number>();
      const controllers = capturedFiles.map(() => new AbortController());
      setUploadState(capturedComposition, { controllers });
      const results = await Promise.allSettled(
        capturedFiles.map((f, index) => uploadFile(f, capturedDirectConversation
          ? { conversationId: capturedDirectConversation }
          : { serverId: capturedServer!, channel: capturedChannel! }, {
            signal: controllers[index].signal,
            onProgress: (loaded, total) => {
              progress.set(index, total > 0 ? loaded / total : 0);
              setUploadState(capturedComposition, { progress: [...progress.values()].reduce((sum, value) => sum + value, 0) / capturedFiles.length });
            },
          })),
      );
      const succeeded = results
        .filter((r): r is PromiseFulfilledResult<import('../../api/types').AttachmentInfo> => r.status === 'fulfilled')
        .map((r) => r.value);
      const failedCount = results.filter((r) => r.status === 'rejected').length;
      if (failedCount > 0) {
        const cancelled = results.every((result) => result.status === 'rejected'
          && result.reason instanceof DOMException && result.reason.name === 'AbortError');
        setUploadState(capturedComposition, { error: cancelled
          ? 'Upload cancelled. Your files are still attached.'
          : `${failedCount} of ${capturedFiles.length} uploads failed. Your files are still attached.` });
      }
      setUploadState(capturedComposition, { controllers: [], uploading: false });
      const current = useChatStore.getState();
      const ui = useUiStore.getState();
      if (current.accountGeneration !== capturedGeneration
          || current.activeAccountId !== capturedAccount
          || ui.activeServer !== capturedServer
          || ui.activeChannel !== capturedChannel
          || ui.activeDirectConversation !== capturedDirectConversation) {
        if (capturedComposition) setCompositionFiles(capturedComposition, capturedFiles);
        return;
      }
      if (failedCount > 0) return;
      if (succeeded.length > 0) {
        attachments = succeeded;
      }
    }

    const accepted = activeDirectConversation && directConversation
      ? sendDirectMessage(activeDirectConversation, directConversation.peer_username, trimmed, attachments)
      : sendMessage(activeServer!, activeChannel!, trimmed, attachments);
    if (accepted) {
      if (key && useComposerStore.getState().drafts[key] === text) setDraft(key, '');
      updatePendingFiles([]);
      if (capturedComposition) clearUploadState(capturedComposition);
      setMentionQuery(null);
    }
  };

  const cancelUploads = () => uploadState.controllers.forEach((controller) => controller.abort());

  useEffect(() => () => {
    Object.values(useComposerStore.getState().uploads)
      .flatMap((upload) => upload.controllers)
      .forEach((controller) => controller.abort());
  }, []);

  const handleKeyDown = (e: KeyboardEvent) => {
    if (composingRef.current || e.nativeEvent.isComposing) return;
    if (slashQuery !== null && slashCandidates.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSlashIndex((index) => (index + 1) % slashCandidates.length);
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSlashIndex((index) => (index - 1 + slashCandidates.length) % slashCandidates.length);
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && text.trim() !== `/${slashCandidates[slashIndex].name}`)) {
        e.preventDefault();
        insertSlashCommand(slashCandidates[slashIndex].name);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setText('');
        return;
      }
    }
    // Mention autocomplete navigation
    if (mentionQuery !== null && mentionCandidates.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setMentionIndex((i) => (i + 1) % mentionCandidates.length);
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setMentionIndex((i) => (i - 1 + mentionCandidates.length) % mentionCandidates.length);
        return;
      }
      if (e.key === 'Tab' || e.key === 'Enter') {
        e.preventDefault();
        insertMention(mentionCandidates[mentionIndex]);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setMentionQuery(null);
        return;
      }
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    } else if (e.key === 'Escape' && replyingTo) {
      if (key) setReplyFor(key, null);
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setText(val);

    // Detect mention trigger: find '@' before cursor
    const cursor = e.target.selectionStart ?? val.length;
    const beforeCursor = val.slice(0, cursor);
    const atIdx = beforeCursor.lastIndexOf('@');
    if (atIdx !== -1) {
      // Only trigger if '@' is at start or preceded by whitespace
      const charBefore = atIdx > 0 ? beforeCursor[atIdx - 1] : ' ';
      if (charBefore === ' ' || charBefore === '\n' || atIdx === 0) {
        const query = beforeCursor.slice(atIdx + 1);
        // Only show autocomplete if no space in the query (single word)
        if (!query.includes(' ')) {
          setMentionQuery(query);
          setMentionStart(atIdx);
        } else {
          setMentionQuery(null);
        }
      } else {
        setMentionQuery(null);
      }
    } else {
      setMentionQuery(null);
    }

    // Send typing indicator (debounced: at most once every 3 seconds)
    if (activeServer && activeChannel) {
      const now = Date.now();
      if (now - lastTypingRef.current > 3000) {
        lastTypingRef.current = now;
        sendTyping(activeServer, activeChannel);
      }
    }
  };

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
      <div className={`relative flex items-center bg-bg-input px-4 ${
        replyingTo && pendingFiles.length === 0 ? 'rounded-b-lg' :
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
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm ${
                  i === mentionIndex
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
