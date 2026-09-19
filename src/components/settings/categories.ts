export type SettingsCategoryId =
  | 'connection'
  | 'appearance'
  | 'playbackBar'
  | 'audio'
  | 'discord'
  | 'scrobbling'
  | 'alerts'
  | 'updates'
  | 'diagnostics'
  | 'about';

export interface SettingsCategory {
  id: SettingsCategoryId;
  label: string;
  title: string;
  hint: string;
  /** Omit the "Reset to defaults" button for this page: Connection because resetting would
   *  wipe live credentials and the channel-group selection with no undo, About because there
   *  is nothing on the page that's a setting. */
  hideReset?: boolean;
  /** Shown indented under the item before it in the settings rail, as its child page. */
  nested?: boolean;
}

export const SETTINGS_CATEGORIES: SettingsCategory[] = [
  {
    id: 'connection',
    label: 'Connection',
    title: 'Connection',
    hint: 'Where Apogee gets your channels from. Changes apply as you make them.',
    hideReset: true,
  },
  { id: 'appearance', label: 'Appearance', title: 'Appearance', hint: 'Changes apply as you make them.' },
  { id: 'playbackBar', label: 'Playback Bar', title: 'Playback Bar', hint: 'Changes apply as you make them.', nested: true },
  {
    id: 'audio',
    label: 'Audio',
    title: 'Audio',
    hint: 'Changes apply as you make them, including to playback in progress.',
  },
  { id: 'discord', label: 'Discord', title: 'Discord', hint: 'Changes apply as you make them.' },
  { id: 'scrobbling', label: 'Scrobbling', title: 'Scrobbling', hint: 'Changes apply as you make them.' },
  { id: 'alerts', label: 'Alerts', title: 'Alerts', hint: 'Changes apply as you make them.' },
  { id: 'updates', label: 'Updates', title: 'Updates', hint: 'Changes apply as you make them.' },
  { id: 'diagnostics', label: 'Diagnostics', title: 'Diagnostics', hint: 'Changes apply as you make them.' },
  { id: 'about', label: 'About', title: 'About', hint: 'Version and legal information.', hideReset: true },
];
