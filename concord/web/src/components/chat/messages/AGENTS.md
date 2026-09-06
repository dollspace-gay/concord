# Message rendering

Separate message interaction, attachments/lightbox, embeds, components, and reactions. MessageList remains the virtualized list owner.

Preserve stable message keys, keyboard access, focus traps, safe external URLs, and private upload handling. Non-component helpers should remain local or live in a .ts utility module.

Run rendering, image, interaction, and accessibility browser tests plus frontend lint/build.
