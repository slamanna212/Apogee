/** Imperative handle a settings panel exposes so the page shell (src/pages/Settings.tsx)
 *  can trigger "Reset to defaults" for whichever category is currently on screen, without
 *  the shell needing to know each panel's internal state shape. Panels whose category hides
 *  the reset button (see SettingsCategory.hideReset in ./categories) don't need to implement this. */
export interface SettingsResetHandle {
  reset: () => void | Promise<void>;
}

/** Every settings panel takes this at minimum: a way to tell the shell a change just
 *  persisted, so it can flash the "Saved" confirmation next to the page title. */
export interface SettingsPanelProps {
  onSaved: () => void;
}
