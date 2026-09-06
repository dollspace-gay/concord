# Composer lifecycle

useMessageComposer owns draft selection, autocomplete, keyboard/IME behavior, uploads, and send callbacks. MessageInput renders its state and controls.

Do not send an upload through a different account or conversation after an await. Preserve cancellation, retained failed files/voice blobs, per-conversation replies, and conditional draft clearing.

Run every composer/upload/account-switch browser harness test and frontend lint/build.
