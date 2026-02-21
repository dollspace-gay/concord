import { useRef, useState, useCallback, useEffect, type KeyboardEvent } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { channelKey } from '../../api/types';
import { uploadFile } from '../../api/client';
import type { MemberInfo } from '../../api/types';
import { GifPicker, isGifPickerAvailable } from './GifPicker';
import { VoiceRecorder } from './VoiceRecorder';
import { EmojiPicker } from './EmojiPicker';

const EMPTY_MEMBERS: MemberInfo[] = [];

export function MessageInput() {
  const [text, setText] = useState('');
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [uploading, setUploading] = useState(false);
  const activeServer = useUiStore((s) => s.activeServer);
  const activeChannel = useUiStore((s) => s.activeChannel);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const sendTyping = useChatStore((s) => s.sendTyping);
  const replyingTo = useChatStore((s) => s.replyingTo);
  const setReplyingTo = useChatStore((s) => s.setReplyingTo);
  const lastTypingRef = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [showGifPicker, setShowGifPicker] = useState(false);
  const [isRecording, setIsRecording] = useState(false);
  const [showEmojiPicker, setShowEmojiPicker] = useState(false);

  // Mention autocomplete state
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionStart, setMentionStart] = useState(0); // cursor position of the '@'
  const key = activeServer && activeChannel ? channelKey(activeServer, activeChannel) : null;
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
  }, [text, mentionStart, mentionQuery]);

  const handleVoiceRecorded = useCallback(async (blob: Blob) => {
    if (!activeServer || !activeChannel) return;
    setIsRecording(false);
    setUploading(true);
    try {
      const file = new File([blob], `voice-message-${Date.now()}.webm`, { type: blob.type });
      const uploaded = await uploadFile(file);
      sendMessage(activeServer, activeChannel, '\u200B', [uploaded]);
    } catch (err) {
      console.error('Voice upload failed:', err);
    } finally {
      setUploading(false);
    }
  }, [activeServer, activeChannel, sendMessage]);

  const handleEmojiSelect = (emoji: string) => {
    setText((prev) => prev + emoji);
    inputRef.current?.focus();
  };

  const handleGifSelect = (url: string) => {
    if (activeServer && activeChannel) {
      sendMessage(activeServer, activeChannel, url);
    }
    setShowGifPicker(false);
  };

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files) {
      setPendingFiles((prev) => [...prev, ...Array.from(e.target.files!)]);
    }
    // Reset so the same file can be re-selected
    e.target.value = '';
  };

  const removePendingFile = (index: number) => {
    setPendingFiles((prev) => prev.filter((_, i) => i !== index));
  };

  const handleSend = async () => {
    const trimmed = text.trim();
    if ((!trimmed && pendingFiles.length === 0) || !activeChannel || !activeServer) return;

    let attachments: import('../../api/types').AttachmentInfo[] | undefined;

    if (pendingFiles.length > 0) {
      setUploading(true);
      const results = await Promise.allSettled(pendingFiles.map((f) => uploadFile(f)));
      const succeeded = results
        .filter((r): r is PromiseFulfilledResult<import('../../api/types').AttachmentInfo> => r.status === 'fulfilled')
        .map((r) => r.value);
      const failedCount = results.filter((r) => r.status === 'rejected').length;
      if (failedCount > 0) {
        console.error(`${failedCount} of ${pendingFiles.length} uploads failed`);
      }
      setUploading(false);
      if (succeeded.length > 0) {
        attachments = succeeded;
      } else if (failedCount > 0) {
        return; // All uploads failed — don't send the message
      }
    }

    sendMessage(activeServer, activeChannel, trimmed || '\u200B', attachments);
    setText('');
    setPendingFiles([]);
    setMentionQuery(null);
  };

  const handleKeyDown = (e: KeyboardEvent) => {
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
      setReplyingTo(null);
    }
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
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

  if (!activeChannel) return null;

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
            onClick={() => setReplyingTo(null)}
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
        {isGifPickerAvailable() && (
          <button
            onClick={() => setShowGifPicker((v) => !v)}
            className="mr-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary"
            title="GIF picker"
          >
            <span className="text-xs font-bold">GIF</span>
          </button>
        )}
        <button
          onClick={() => setShowEmojiPicker((v) => !v)}
          className="mr-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary"
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
        <input
          ref={inputRef}
          type="text"
          value={text}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          placeholder={`Message ${activeChannel}`}
          className="flex-1 bg-transparent py-3 text-text-primary placeholder-text-muted outline-none"
        />
        <button
          onClick={() => setIsRecording(true)}
          className="ml-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary"
          title="Record voice message"
        >
          <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4M12 15a3 3 0 003-3V5a3 3 0 00-6 0v7a3 3 0 003 3z" />
          </svg>
        </button>
        <button
          onClick={handleSend}
          disabled={(!text.trim() && pendingFiles.length === 0) || uploading}
          className="ml-2 rounded p-1.5 text-text-muted transition-colors hover:text-text-primary disabled:opacity-30"
        >
          {uploading ? (
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
