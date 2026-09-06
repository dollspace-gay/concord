import type { EmbedInfo, RichEmbedInfo } from '../../../api/types';
import { useChatStore } from '../../../stores/chatStore';
import { safeExternalHttpsUrl } from '../../../utils/externalUrl';
import { ExternalImage } from '../../ExternalImage';
import { FormattedMessage } from '../FormattedMessage';

export function LinkEmbed({ embed, messageId }: { embed: EmbedInfo; messageId: string }) {
  const activeAccountId = useChatStore((state) => state.activeAccountId);
  const url = safeExternalHttpsUrl(embed.url);
  return (
    <article
      className="flex max-w-[480px] overflow-hidden rounded border-l-4 border-blue-500 bg-bg-secondary transition-colors hover:bg-bg-hover"
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1 p-3">
        {embed.site_name && (
          <span className="text-xs text-text-muted">{embed.site_name}</span>
        )}
        {embed.title && url
          ? <a href={url} target="_blank" rel="noopener noreferrer" className="text-sm font-semibold text-blue-400 hover:underline">{embed.title}</a>
          : embed.title && <span className="text-sm font-semibold text-text-primary">{embed.title}</span>}
        {embed.description && (
          <span className="line-clamp-3 text-sm text-text-secondary">{embed.description}</span>
        )}
      </div>
      {embed.image_url && (
        <ExternalImage
          src={embed.image_url}
          alt=""
          label="link preview"
          className="h-20 w-20 shrink-0 object-cover"
          privacyScopeKey={`${activeAccountId ?? ''}:${messageId}:link-preview`}
        />
      )}
    </article>
  );
}

export function RichEmbed({ embed, messageId }: { embed: RichEmbedInfo; messageId: string }) {
  const activeAccountId = useChatStore((state) => state.activeAccountId);
  const privacyScopeKey = `${activeAccountId ?? ''}:${messageId}`;
  const url = safeExternalHttpsUrl(embed.url);
  const imageUrl = safeExternalHttpsUrl(embed.image_url);
  const thumbnailUrl = safeExternalHttpsUrl(embed.thumbnail_url);
  const authorUrl = safeExternalHttpsUrl(embed.author?.url);
  const authorIconUrl = safeExternalHttpsUrl(embed.author?.icon_url);
  const footerIconUrl = safeExternalHttpsUrl(embed.footer?.icon_url);
  const title = embed.title && url
    ? <a href={url} target="_blank" rel="noopener noreferrer" className="font-semibold text-blue-400 hover:underline">{embed.title}</a>
    : embed.title && <div className="font-semibold text-text-primary">{embed.title}</div>;
  return (
    <article
      className="overflow-hidden rounded border-l-4 bg-bg-secondary p-3 text-sm"
      style={{ borderLeftColor: embed.color || '#5865f2' }}
      aria-label={embed.title ? `Embed: ${embed.title}` : 'Message embed'}
    >
      <div className="flex gap-3">
        <div className="min-w-0 flex-1 space-y-2">
          {embed.author && (
            <div className="flex items-center gap-2 text-xs font-medium text-text-primary">
              {authorIconUrl && <ExternalImage src={authorIconUrl} alt="" label="author icon" className="h-5 w-5 rounded-full object-cover" privacyScopeKey={privacyScopeKey} />}
              {authorUrl
                ? <a href={authorUrl} target="_blank" rel="noopener noreferrer" className="hover:underline">{embed.author.name}</a>
                : <span>{embed.author.name}</span>}
            </div>
          )}
          {title}
          {embed.description && <FormattedMessage content={embed.description} />}
          {embed.fields && embed.fields.length > 0 && (
            <dl className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {embed.fields.map((field, index) => (
                <div key={`${field.name}-${index}`} className={field.inline ? '' : 'sm:col-span-2'}>
                  <dt className="font-semibold text-text-primary">{field.name}</dt>
                  <dd className="text-text-secondary"><FormattedMessage content={field.value} /></dd>
                </div>
              ))}
            </dl>
          )}
          {imageUrl && <ExternalImage src={imageUrl} alt="" label="embed image" className="max-h-80 max-w-full rounded object-contain" privacyScopeKey={privacyScopeKey} />}
          {embed.footer && (
            <footer className="flex items-center gap-2 text-xs text-text-muted">
              {footerIconUrl && <ExternalImage src={footerIconUrl} alt="" label="footer icon" className="h-5 w-5 rounded-full object-cover" privacyScopeKey={privacyScopeKey} />}
              <span>{embed.footer.text}</span>
              {embed.timestamp && <time dateTime={embed.timestamp}> · {new Date(embed.timestamp).toLocaleString()}</time>}
            </footer>
          )}
        </div>
        {thumbnailUrl && <ExternalImage src={thumbnailUrl} alt="" label="embed thumbnail" className="h-20 w-20 shrink-0 rounded object-cover" privacyScopeKey={privacyScopeKey} />}
      </div>
    </article>
  );
}
