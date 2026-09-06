

export interface RichEmbedInfo {
  title?: string | null;
  description?: string | null;
  url?: string | null;
  color?: string | null;
  fields?: EmbedField[] | null;
  footer?: { text: string; icon_url?: string | null } | null;
  author?: { name: string; url?: string | null; icon_url?: string | null } | null;
  thumbnail_url?: string | null;
  image_url?: string | null;
  timestamp?: string | null;
}

export interface EmbedField {
  name: string;
  value: string;
  inline?: boolean;
}

export type MessageComponent =
  | { type: 'action_row'; components: MessageComponent[] }
  | { type: 'button'; custom_id: string; label: string; style?: string; emoji?: string | null; disabled?: boolean }
  | { type: 'select_menu'; custom_id: string; placeholder?: string | null; min_values?: number; max_values?: number; options: SelectOption[] };

export interface SelectOption {
  label: string;
  value: string;
  description?: string | null;
  emoji?: string | null;
  default?: boolean;
}
