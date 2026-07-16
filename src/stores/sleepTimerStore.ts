import { create } from 'zustand';
import { usePlayerStore } from './playerStore';
import { useSettingsStore } from './settingsStore';

interface SleepTimerState {
  remainingSeconds: number | null;
  start: (minutes: number) => void;
  cancel: () => void;
}

let tickTimer: ReturnType<typeof setInterval> | null = null;

function stopTick() {
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
}

export const useSleepTimerStore = create<SleepTimerState>((set, get) => ({
  remainingSeconds: null,

  start(minutes) {
    stopTick();
    set({ remainingSeconds: Math.round(minutes * 60) });
    void useSettingsStore.getState().update({ lastSleepTimerMinutes: minutes });
    tickTimer = setInterval(() => {
      const next = (get().remainingSeconds ?? 0) - 1;
      if (next <= 0) {
        stopTick();
        set({ remainingSeconds: null });
        void usePlayerStore.getState().stop();
        return;
      }
      set({ remainingSeconds: next });
    }, 1000);
  },

  cancel() {
    stopTick();
    set({ remainingSeconds: null });
  },
}));
