import { Checkbox, Group, MultiSelect } from '@mantine/core';
import type { XtreamCategory } from '../types/xtream';

interface ChannelGroupSelectorProps {
  categories: XtreamCategory[];
  value: string[];
  onChange: (ids: string[]) => void;
  disabled?: boolean;
}

export function ChannelGroupSelector({ categories, value, onChange, disabled }: ChannelGroupSelectorProps) {
  return (
    <MultiSelect
      label="Channel groups"
      placeholder={categories.length === 0 ? 'Run Test connection to load groups' : 'Select one or more groups'}
      data={categories.map((c) => ({ value: c.category_id, label: c.category_name }))}
      value={value}
      onChange={onChange}
      disabled={disabled ?? categories.length === 0}
      searchable
      clearable
      renderOption={({ option, checked }) => (
        <Group flex="1" gap="sm" wrap="nowrap">
          <Checkbox checked={checked} onChange={() => {}} tabIndex={-1} style={{ pointerEvents: 'none' }} />
          {option.label}
        </Group>
      )}
    />
  );
}
