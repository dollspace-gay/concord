# Incoming webhooks

An incoming webhook credential is shown once when the webhook is created. The
credential represents a dedicated bot principal and is restricted to the
webhook's original server and channel. Moving an incoming webhook between
channels is unsupported; create a new webhook when the destination changes.

Send a message with:

```http
POST /api/webhooks/{webhook_id}/{credential}
Content-Type: application/json

{
  "content": "Build completed",
  "idempotency_key": "build-1842-notification"
}
```

`idempotency_key` is required. Repeating the same key and content returns the
original committed message receipt with `replayed: true`. Reusing the key with
different content is rejected. A successful request returns `201 Created` and
the canonical message receipt; this replaces the earlier `204 No Content`
response. The message is committed through the normal authorization, AutoMod,
rate-limit, slow-mode, event, and delivery-outbox path before success is
returned.

The request cannot override the bot username or avatar. Revoking its
installation or deleting the webhook immediately prevents further sends.
