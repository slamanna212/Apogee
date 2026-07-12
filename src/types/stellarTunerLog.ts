export interface StellarStation {
  id: string;
  name: string;
  channel_number: number;
  artist: string;
  title: string;
  album: string;
  cut_type: string;
  artwork_url: string;
  itunes_id: string;
}

export interface StellarNowPlayingResponse {
  updated_utc: string;
  poll_interval_seconds: number;
  station_count: number;
  stations: Record<string, StellarStation>;
}

export interface NowPlayingEntry {
  channelNumber: number;
  channelName: string;
  artist: string;
  title: string;
  album?: string;
  artworkUrl?: string;
  updatedAt: string;
}

export interface StellarChannelCategory {
  name: string;
  order: number;
  is_primary: boolean;
}

export interface StellarChannelLogo {
  url: string;
  width: number;
  height: number;
}

export interface StellarChannelLogos {
  color_dark?: StellarChannelLogo;
  color_dark_square?: StellarChannelLogo;
  color_dark_alexa?: StellarChannelLogo;
  background_wide?: StellarChannelLogo;
  background_thumb?: StellarChannelLogo;
  background_square?: StellarChannelLogo;
  background?: StellarChannelLogo;
  list_view?: StellarChannelLogo;
  list_view_square?: StellarChannelLogo;
  white?: StellarChannelLogo;
  white_square?: StellarChannelLogo;
  [key: string]: StellarChannelLogo | undefined;
}

export interface StellarChannel {
  id: string;
  name: string;
  marketing_name: string;
  channel_number: number;
  categories: StellarChannelCategory[];
  logos?: StellarChannelLogos;
  dark_bg_color?: string;
  medium_description?: string;
  long_description?: string;
  streaming_name?: string;
  twitter?: string;
  facebook?: string;
  email?: string;
  phone?: string;
}

export interface StellarChannelsResponse {
  channel_count: number;
  channels: StellarChannel[] | Record<string, StellarChannel>;
}

export interface StellarHistoryEntry {
  played_at: string;
  artist: string;
  title: string;
  album?: string;
  artwork_url?: string;
  cut_type?: string;
}

export interface StellarHistoryResponse {
  channel_id: string;
  plays: StellarHistoryEntry[];
}
