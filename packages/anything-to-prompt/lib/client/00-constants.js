const React = require('react');
const {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} = React;
const h = React.createElement;
const ReactDOM = (() => {
  try {
    return require('react-dom');
  } catch {
    return null;
  }
})();

const DSHPrimitives = (() => {
  try {
    return require('@deepseek-ai/dsh-client-ui-primitives');
  } catch {
    return null;
  }
})();

const CARD_RADIUS_PX = 24;
const COLLAPSED_LIP_PX = 56;
const BAR_STAGE_HEIGHT_DIP = 680;
const TUCK_OVERLAP_PX = 0;
const HOVER_EXPAND_DELAY_MS = 300;
const DEFAULT_EXPANDED_PX = 280;
const MIN_EXPANDED_PX = 160;
const MAX_EXPANDED_PX = 560;
const DEFAULT_BAR_WIDTH_PX = 760;
const MIN_BAR_WIDTH_PX = 480;
const MAX_BAR_WIDTH_PX = 960;
const DEFAULT_SIDE_WIDTH_PX = 420;
const MIN_SIDE_WIDTH_PX = 360;
const MAX_SIDE_WIDTH_PX = 960;

const STORAGE_KEY_PINNED = 'dsh_card_dock:pinned';
const STORAGE_KEY_HEIGHT = 'dsh_card_dock:expanded_height';
const STORAGE_KEY_BAR_WIDTH = 'dsh_card_dock:bar_width';
const STORAGE_KEY_SIDE_WIDTH = 'dsh_card_dock:side_width';
const STORAGE_KEY_LAST_SESSION = 'dsh_card_dock:bar_last_session';
const STORAGE_KEY_ACTIVE_HARNESS = 'dsh_card_dock:active_harness';
const STORAGE_KEY_CLAUDE_PERM = 'dsh_card_dock:claude_perm_mode';
const STORAGE_KEY_THEME_PREF = 'dsh_card_dock:theme_pref';

