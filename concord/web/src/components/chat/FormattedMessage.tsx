import { useMemo } from 'react';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';

interface FormattedMessageProps {
  content: string;
}

/** Token types for the inline parser. */
type Token =
  | { type: 'text'; value: string }
  | { type: 'bold'; children: Token[] }
  | { type: 'italic'; children: Token[] }
  | { type: 'strikethrough'; children: Token[] }
  | { type: 'code'; value: string }
  | { type: 'spoiler'; children: Token[] }
  | { type: 'mention'; value: string }
  | { type: 'custom_emoji'; name: string }
  | { type: 'sticker'; name: string };

/**
 * Parse inline markdown tokens from a string.
 * Supports: **bold**, *italic*, ~~strikethrough~~, `code`, ||spoiler||
 */
function parseInline(text: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;

  while (i < text.length) {
    // Inline code: `...`
    if (text[i] === '`') {
      const end = text.indexOf('`', i + 1);
      if (end !== -1) {
        tokens.push({ type: 'code', value: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }

    // Bold: **...**
    if (text[i] === '*' && text[i + 1] === '*') {
      const end = text.indexOf('**', i + 2);
      if (end !== -1) {
        tokens.push({ type: 'bold', children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    // Italic: *...*
    if (text[i] === '*' && text[i + 1] !== '*') {
      const end = text.indexOf('*', i + 1);
      if (end !== -1 && text[end + 1] !== '*') {
        tokens.push({ type: 'italic', children: parseInline(text.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }

    // Strikethrough: ~~...~~
    if (text[i] === '~' && text[i + 1] === '~') {
      const end = text.indexOf('~~', i + 2);
      if (end !== -1) {
        tokens.push({ type: 'strikethrough', children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    // Spoiler: ||...||
    if (text[i] === '|' && text[i + 1] === '|') {
      const end = text.indexOf('||', i + 2);
      if (end !== -1) {
        tokens.push({ type: 'spoiler', children: parseInline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }

    // Mention: @username, @everyone, @here
    if (text[i] === '@') {
      const match = text.slice(i).match(/^@(\w+)/);
      if (match) {
        tokens.push({ type: 'mention', value: match[0] });
        i += match[0].length;
        continue;
      }
    }

    // Custom emoji: :name:
    if (text[i] === ':') {
      const match = text.slice(i).match(/^:(\w{2,32}):/);
      if (match) {
        tokens.push({ type: 'custom_emoji', name: match[1] });
        i += match[0].length;
        continue;
      }
    }

    // Sticker: [sticker:name]
    if (text[i] === '[') {
      const match = text.slice(i).match(/^\[sticker:(\w{2,32})\]/);
      if (match) {
        tokens.push({ type: 'sticker', name: match[1] });
        i += match[0].length;
        continue;
      }
    }

    // Plain text — collect until next special character
    let j = i + 1;
    while (j < text.length && !['*', '~', '`', '|', '@', ':', '['].includes(text[j])) {
      j++;
    }
    tokens.push({ type: 'text', value: text.slice(i, j) });
    i = j;
  }

  return tokens;
}

/** Render parsed tokens to React elements. */
function renderTokens(tokens: Token[], keyPrefix = ''): React.ReactNode[] {
  return tokens.map((token, idx) => {
    const key = `${keyPrefix}${idx}`;
    switch (token.type) {
      case 'text':
        return <span key={key}>{token.value}</span>;
      case 'bold':
        return <strong key={key}>{renderTokens(token.children, `${key}-`)}</strong>;
      case 'italic':
        return <em key={key}>{renderTokens(token.children, `${key}-`)}</em>;
      case 'strikethrough':
        return <del key={key}>{renderTokens(token.children, `${key}-`)}</del>;
      case 'code':
        return (
          <code key={key} className="rounded bg-bg-secondary px-1 py-0.5 text-sm font-mono">
            {token.value}
          </code>
        );
      case 'spoiler':
        return <SpoilerSpan key={key}>{renderTokens(token.children, `${key}-`)}</SpoilerSpan>;
      case 'mention':
        return <MentionSpan key={key} value={token.value} />;
      case 'custom_emoji':
        return <CustomEmojiSpan key={key} name={token.name} />;
      case 'sticker':
        return <StickerSpan key={key} name={token.name} />;
    }
  });
}

function SpoilerSpan({ children }: { children: React.ReactNode }) {
  return (
    <span
      className="cursor-pointer rounded bg-text-muted text-transparent transition-colors hover:bg-transparent hover:text-text-primary"
    >
      {children}
    </span>
  );
}

function MentionSpan({ value }: { value: string }) {
  const isGlobal = value === '@everyone' || value === '@here';
  return (
    <span
      className={`rounded px-0.5 font-medium ${
        isGlobal
          ? 'bg-yellow-500/20 text-yellow-300'
          : 'bg-blue-500/20 text-blue-300 hover:bg-blue-500/30 cursor-pointer'
      }`}
    >
      {value}
    </span>
  );
}

function CustomEmojiSpan({ name }: { name: string }) {
  const activeServer = useUiStore((s) => s.activeServer);
  const url = useChatStore((s) => {
    // Priority 1: current server emoji
    if (activeServer) {
      const local = s.customEmoji[activeServer]?.[name];
      if (local) return local;
    }
    // Priority 2: cross-server emoji from all user's servers
    const cross = s.allUserEmoji.find((e) => e.name === name);
    return cross?.image_url;
  });

  if (url) {
    return (
      <img
        src={url}
        alt={`:${name}:`}
        title={`:${name}:`}
        className="inline-block h-6 w-6 align-text-bottom"
      />
    );
  }
  return <span>:{name}:</span>;
}

function StickerSpan({ name }: { name: string }) {
  const activeServer = useUiStore((s) => s.activeServer);
  const url = useChatStore((s) => {
    if (!activeServer) return undefined;
    const sticker = s.stickers[activeServer]?.find((st) => st.name === name);
    return sticker?.image_url;
  });

  if (url) {
    return (
      <img
        src={url}
        alt={`[sticker:${name}]`}
        title={name}
        className="inline-block h-32 w-32"
      />
    );
  }
  return <span>[sticker:{name}]</span>;
}

/**
 * Split content into blocks: code blocks (```...```) and inline paragraphs.
 * Also handles blockquotes (lines starting with >).
 */
function parseBlocks(content: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const lines = content.split('\n');
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Code block: ```...```
    if (line.trimStart().startsWith('```')) {
      const lang = line.trimStart().slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith('```')) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // skip closing ```
      nodes.push(
        <pre key={nodes.length} className="my-1 overflow-x-auto rounded bg-bg-secondary p-3 text-sm">
          <code>{codeLines.join('\n')}{lang ? '' : ''}</code>
        </pre>,
      );
      continue;
    }

    // Blockquote: lines starting with >
    if (line.startsWith('>')) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith('>')) {
        quoteLines.push(lines[i].replace(/^>\s?/, ''));
        i++;
      }
      const quoteContent = quoteLines.join('\n');
      nodes.push(
        <blockquote
          key={nodes.length}
          className="my-1 border-l-4 border-text-muted pl-3 text-text-muted"
        >
          {renderTokens(parseInline(quoteContent))}
        </blockquote>,
      );
      continue;
    }

    // Regular line: parse inline formatting
    if (line.trim() === '') {
      nodes.push(<br key={nodes.length} />);
    } else {
      nodes.push(
        <span key={nodes.length}>
          {renderTokens(parseInline(line))}
          {i < lines.length - 1 ? '\n' : ''}
        </span>,
      );
    }
    i++;
  }

  return nodes;
}

/**
 * Renders message content with Discord-style formatting:
 * - **bold**, *italic*, ~~strikethrough~~, `inline code`
 * - ```code blocks```
 * - > blockquotes
 * - ||spoiler||
 */
export function FormattedMessage({ content }: FormattedMessageProps) {
  const rendered = useMemo(() => parseBlocks(content), [content]);

  return (
    <span className="whitespace-pre-wrap break-words text-text-secondary">
      {rendered}
    </span>
  );
}
