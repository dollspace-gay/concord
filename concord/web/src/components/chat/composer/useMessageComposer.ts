import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type SetStateAction } from 'react';
import { uploadFile } from '../../../api/client';
import type { MemberInfo, SlashCommandInfo } from '../../../api/types';
import { channelKey } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { useComposerStore } from '../../../stores/composerStore';
import { useUiStore } from '../../../stores/uiStore';


const EMPTY_MEMBERS: MemberInfo[] = [];

const EMPTY_FILES: File[] = [];

const EMPTY_SLASH_COMMANDS: SlashCommandInfo[] = [];

const EMPTY_UPLOAD = { uploading: false, progress: 0, error: null, voiceRetryBlob: null, controllers: [] } as const;
export function useMessageComposer() {

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

  const slashSelectionKey = `${compositionKey}:${slashQuery}:${slashCandidates.length}`;
  const [previousSlashSelectionKey, setPreviousSlashSelectionKey] = useState(slashSelectionKey);
  if (previousSlashSelectionKey !== slashSelectionKey) {
    setPreviousSlashSelectionKey(slashSelectionKey);
    setSlashIndex(0);
  }

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

  const mentionSelectionKey = `${compositionKey}:${mentionQuery}:${mentionCandidates.length}`;
  const [previousMentionSelectionKey, setPreviousMentionSelectionKey] = useState(mentionSelectionKey);
  if (previousMentionSelectionKey !== mentionSelectionKey) {
    setPreviousMentionSelectionKey(mentionSelectionKey);
    setMentionIndex(0);
  }

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

    let attachments: import('../../../api/types').AttachmentInfo[] | undefined;

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
        .filter((r): r is PromiseFulfilledResult<import('../../../api/types').AttachmentInfo> => r.status === 'fulfilled')
        .map((r) => r.value);
      const failedCount = results.filter((r) => r.status === 'rejected').length;
      if (failedCount > 0) {
        const cancelled = results.every((result) => result.status === 'rejected'
          && result.reason instanceof DOMException && result.reason.name === 'AbortError');
        setUploadState(capturedComposition, {
          error: cancelled
            ? 'Upload cancelled. Your files are still attached.'
            : `${failedCount} of ${capturedFiles.length} uploads failed. Your files are still attached.`
        });
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


  return {
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
  };
}
