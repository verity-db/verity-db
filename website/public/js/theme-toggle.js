/**
 * Theme Toggle - Light/Dark mode with localStorage persistence
 *
 * Follows Albers' principle: grayscale maintains visual hierarchy
 * across both themes through lightness relationships.
 */

(function () {
  'use strict';

  const STORAGE_KEY = 'theme';
  const THEMES = ['light', 'dark'];

  /**
   * Get the current theme from DOM
   */
  function getCurrentTheme() {
    return document.documentElement.getAttribute('data-theme') || 'light';
  }

  /**
   * Set theme on document and persist to localStorage
   */
  function setTheme(theme) {
    if (!THEMES.includes(theme)) return;

    document.documentElement.setAttribute('data-theme', theme);
    document.documentElement.style.colorScheme = theme;
    localStorage.setItem(STORAGE_KEY, theme);

    // Update toggle button aria-label and icon visibility
    updateToggleButton(theme);
  }

  /**
   * Toggle between light and dark themes
   */
  function toggleTheme() {
    const current = getCurrentTheme();
    const next = current === 'light' ? 'dark' : 'light';
    setTheme(next);
  }

  /**
   * Update the toggle button's aria-label
   * Icon visibility is handled by CSS based on data-theme attribute
   */
  function updateToggleButton(theme) {
    const button = document.querySelector('.theme-toggle');
    if (!button) return;

    const isDark = theme === 'dark';
    button.setAttribute(
      'aria-label',
      isDark ? 'Switch to light mode' : 'Switch to dark mode'
    );
  }

  /**
   * Initialize theme toggle functionality
   */
  function init() {
    // Set initial button state
    updateToggleButton(getCurrentTheme());

    // Attach click handler to toggle button
    const button = document.querySelector('.theme-toggle');
    if (button) {
      button.addEventListener('click', toggleTheme);
    }

    // Listen for system preference changes
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    mediaQuery.addEventListener('change', (e) => {
      // Only auto-switch if user hasn't set a preference
      const stored = localStorage.getItem(STORAGE_KEY);
      if (!stored) {
        setTheme(e.matches ? 'dark' : 'light');
      }
    });
  }

  // Initialize when DOM is ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }

  // Expose toggle function globally for inline handlers if needed
  window.toggleTheme = toggleTheme;
})();
