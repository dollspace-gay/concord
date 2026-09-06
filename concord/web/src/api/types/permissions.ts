

export interface RoleInfo {
  id: string;
  server_id: string;
  name: string;
  color?: string | null;
  icon_url?: string | null;
  position: number;
  permissions: number;
  is_default: boolean;
}

export interface CategoryInfo {
  id: string;
  server_id: string;
  name: string;
  position: number;
}

export interface ChannelPositionInfo {
  id: string;
  category_id?: string | null;
  position: number;
}
