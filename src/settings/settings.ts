import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { barBackground, patternImage, PRESETS, withAlpha } from "../dock/appearance";
import {
  AppConfig,
  AppWindow,
  DockConfig,
  SavedTheme,
  Theme,
  WidgetId,
  ScreenInfo,
  WindowsLook,
  WindowsTweaks,
} from "../config";
import { settingsPalette } from "./palette";
import { batteryIcon, ICONS, wifiIcon } from "../dock/widgets";

type Control = HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement;
type Values = Record<string, string | number | boolean>;

const SAVE_DELAY_MS = 120;
const REFRESH_RUNNING_MS = 2000;
const PAGE_STORAGE_KEY = "settings-page";
const DEFAULT_PAGE = "themes";
const PREVIEW_APPS = 5;
const TWEAK_SETTLE_MS = 300;
const PREVIEW_MARGIN = 20;
const START_LOGO = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3h8.500v8.500H3zM12.500 3H21v8.500h-8.500zM3 12.500h8.500V21H3zM12.500 12.500H21V21h-8.500z"/></svg>`;

const FORMATS: Record<string, (value: number) => string> = {
  percent: (value) => `${Math.round(value * 100)}%`,
  percent100: (value) => `${value}%`,
  px: (value) => `${value}px`,
  deg: (value) => `${value}°`,
  times: (value) => `${value.toFixed(2)}×`,
};

const WIDGET_INFO: Record<WidgetId, { name: string; description: string }> = {
  clock: { name: "Relógio", description: "Clique abre o calendário" },
  volume: { name: "Volume", description: "Ajuste com a rodinha do mouse" },
  network: { name: "Rede", description: "Wi-Fi ou cabo" },
  battery: { name: "Bateria", description: "Em aparelhos com bateria" },
  system: { name: "CPU e memória", description: "Clique abre o Gerenciador de Tarefas" },
  media: { name: "Música", description: "Aparece com mídia aberta" },
};

const form = document.querySelector<HTMLFormElement>("#form")!;
const appsEl = document.querySelector<HTMLUListElement>("#apps")!;
const runningEl = document.querySelector<HTMLElement>("#running")!;
const themeName = document.querySelector<HTMLInputElement>("#theme-name")!;
const stage = document.querySelector<HTMLElement>("#stage")!;
const preview = document.querySelector<HTMLElement>("#preview")!;

let appConfig: AppConfig;
/** The dock's section of `appConfig`, which most of this window edits. */
let config: DockConfig;
let saveTimer: number | undefined;
let runningExes: string[] = [];
let previewContent = "";
let windowColors = "";
let lookRequest = 0;
const icons = new Map<string, Promise<string | null>>();

function iconFor(path: string): Promise<string | null> {
  if (!icons.has(path)) icons.set(path, invoke("get_icon", { path }));
  return icons.get(path)!;
}

function iconImage(path: string): HTMLImageElement {
  const img = document.createElement("img");
  img.alt = "";
  iconFor(path).then((src) => {
    if (src) img.src = src;
  });
  return img;
}

function scheduleSave() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke("save_config", { config: appConfig }).catch(showError);
  }, SAVE_DELAY_MS);
}

function showError(error: unknown) {
  document.querySelector("#load-error-text")!.textContent = String(error);
  document.querySelector<HTMLElement>("#load-error")!.hidden = false;
}

function target(control: Control): Values {
  const { app, win, theme, clock } = control.dataset;
  const owner = app
    ? appConfig
    : win
      ? appConfig.windows
      : theme
        ? config.theme
        : clock
          ? config.clock
          : config;
  return owner as unknown as Values;
}

function controlKey(control: Control): string {
  const { app, win, theme, clock, key } = control.dataset;
  return app ?? win ?? theme ?? clock ?? key!;
}

function showOutput(control: Control) {
  const output = control.parentElement?.querySelector("output");
  const format = FORMATS[control.dataset.format ?? ""];
  if (output && format) output.textContent = format(Number(control.value));
  if (control instanceof HTMLInputElement && control.type === "range") {
    const span = Number(control.max) - Number(control.min);
    const filled = ((Number(control.value) - Number(control.min)) / span) * 100;
    control.style.setProperty("--fill", `${filled}%`);
  }
}

function controls(): Control[] {
  return Array.from(
    form.querySelectorAll<Control>(
      "[data-app], [data-win], [data-key], [data-theme], [data-clock]",
    ),
  );
}

function refreshControls() {
  for (const control of controls()) {
    const value = target(control)[controlKey(control)];
    if (control instanceof HTMLInputElement && control.type === "checkbox") {
      control.checked = Boolean(value);
    } else if (control instanceof HTMLInputElement && control.type === "radio") {
      control.checked = control.value === String(value);
    } else {
      control.value = String(value);
    }
    showOutput(control);
  }
  renderThemes();
  refreshLook();
}

/** Keeps the window's colors, the preview and the optional controls in step. */
function refreshLook() {
  applyPalette();
  refreshOptionalControls();
  document.querySelector<HTMLElement>("#monitor")!.dataset.align = config.alignment;
  renderPreview();
  renderWindowsLook().catch(showError);
}

/** Shows the mode and accent the Windows module applies, as Rust works them out. */
async function renderWindowsLook() {
  const request = ++lookRequest;
  const look = await invoke<WindowsLook>("get_windows_look", { config: appConfig });
  // An older answer must not replace the one for the latest change.
  if (request !== lookRequest) return;
  document.querySelector<HTMLElement>("#win-look-swatch")!.style.background = look.accent;
  document.querySelector("#win-look-text")!.textContent =
    `${look.dark ? "Modo escuro" : "Modo claro"} com destaque ${look.accent}`;
}

function applyPalette() {
  const palette = settingsPalette(config.theme);
  const root = document.documentElement;
  for (const [name, value] of Object.entries(palette.variables)) {
    root.style.setProperty(name, value);
  }
  root.style.colorScheme = palette.dark ? "dark" : "light";

  const colors = `${palette.background}|${palette.text}|${palette.dark}`;
  if (colors !== windowColors) {
    windowColors = colors;
    invoke("set_settings_colors", {
      caption: palette.background,
      text: palette.text,
      dark: palette.dark,
    }).catch(showError);
  }

  for (const input of document.querySelectorAll<HTMLInputElement>(".tiles input")) {
    const tile = input.nextElementSibling as HTMLElement;
    const image = patternImage(input.value, palette.text);
    tile.style.backgroundImage = image ?? "none";
    tile.style.backgroundSize = "20px";
  }
}

/** Settings that only matter once another one is turned on stay out of the way. */
function refreshOptionalControls() {
  const values = { ...config, ...config.theme } as unknown as Values;
  for (const [key, value] of Object.entries(appConfig.windows)) {
    values[`win.${key}`] = value;
  }
  const off = new Set(["none", "off"]);
  for (const element of document.querySelectorAll<HTMLElement>("[data-show-when]")) {
    // "!key" shows the element while the setting is off instead.
    const rule = element.dataset.showWhen!;
    const inverted = rule.startsWith("!");
    const value = values[inverted ? rule.slice(1) : rule];
    const on = Boolean(value) && !off.has(String(value));
    element.hidden = on === inverted;
  }
}

function clockText(): string {
  const { showDate, showSeconds, hour12 } = config.clock;
  const now = new Date();
  const time = now.toLocaleTimeString("pt-BR", {
    hour: "2-digit",
    minute: "2-digit",
    second: showSeconds ? "2-digit" : undefined,
    hour12,
  });
  if (!showDate) return time;
  const date = now.toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit" });
  return `${time} · ${date}`;
}

function previewWidget(id: WidgetId): string {
  switch (id) {
    case "clock":
      return `<span>${clockText()}</span>`;
    case "volume":
      return ICONS.speaker;
    case "network":
      return wifiIcon(5);
    case "battery":
      return batteryIcon(80, false);
    case "system":
      return `<span>CPU • RAM</span>`;
    case "media":
      return ICONS.play;
  }
}

function previewTile(className: string, path?: string): HTMLElement {
  const tile = document.createElement("div");
  tile.className = `tile ${className}`;
  if (path) tile.append(iconImage(path));
  else tile.innerHTML = START_LOGO;
  return tile;
}

function renderPreviewContent() {
  const paths = [
    ...new Set([...config.apps.map((app) => app.path), ...runningExes]),
  ].slice(0, PREVIEW_APPS);
  const widgets = config.widgets
    .filter((widget) => widget.enabled)
    .map((widget) => previewWidget(widget.id));

  const content = [config.showStartButton, paths, widgets].join("|");
  if (content === previewContent) return;
  previewContent = content;

  const items: HTMLElement[] = paths.map((path, index) =>
    previewTile(index === 0 ? "open focused" : "open", path),
  );
  if (config.showStartButton) items.unshift(previewTile("start"));
  if (widgets.length > 0) {
    const capsule = document.createElement("div");
    capsule.className = "capsule";
    capsule.innerHTML = widgets.join("");
    items.push(capsule);
  }
  preview.replaceChildren(...items);
}

function fitPreview() {
  preview.style.transform = "";
  const scale = Math.min(
    1,
    (stage.clientWidth - PREVIEW_MARGIN) / preview.offsetWidth,
    (stage.clientHeight - PREVIEW_MARGIN) / preview.offsetHeight,
  );
  if (scale < 1) preview.style.transform = `scale(${scale})`;
}

function renderPreview() {
  const { theme } = config;
  const { style } = preview;
  style.background = barBackground(theme);
  style.borderColor = withAlpha(theme.borderColor, theme.borderOpacity);
  style.borderWidth = `${theme.borderWidth}px`;
  style.borderRadius = `${theme.radius}px`;
  style.padding = `${theme.padding}px`;
  style.gap = `${theme.gap}px`;
  style.boxShadow = `0 0 ${theme.glowSize}px ${theme.glowSize > 0 ? theme.glowColor : "transparent"}`;
  style.fontFamily = `"${theme.font}", "Segoe UI", sans-serif`;
  style.color = theme.widgetColor;
  style.setProperty("--icon-size", `${config.iconSize}px`);
  style.setProperty("--dot", theme.dotColor);
  style.setProperty("--dot-active", theme.activeColor);
  preview.classList.toggle("no-bar", theme.barOpacity < 0.25 && !theme.barImage);
  renderPreviewContent();
  fitPreview();
}

/** Applies right away while the slider moves, and keeps the value for next time. */
async function setBrightness(id: string, percent: number, slider: HTMLInputElement) {
  const applied = await invoke<number>("set_brightness", { id, percent });
  appConfig.display.brightness[id] = applied;
  if (applied !== percent) {
    // Windows refused to go that dark, so the slider shows what it allowed.
    slider.value = String(applied);
    showOutput(slider);
  }
  scheduleSave();
}

async function renderScreens() {
  const screens = await invoke<ScreenInfo[]>("get_screens");
  const container = document.querySelector("#screens")!;
  if (screens.length === 0) {
    container.innerHTML = `<p class="hint">Nenhuma tela encontrada.</p>`;
    return;
  }
  container.replaceChildren(
    ...screens.map((screen) => {
      const row = document.createElement("label");
      row.className = "slider";
      const name = document.createElement("span");
      name.textContent = screen.name;
      if (screen.primary) {
        const badge = document.createElement("small");
        badge.textContent = "principal";
        name.append(badge);
      }
      const slider = document.createElement("input");
      slider.type = "range";
      slider.min = "50";
      slider.max = "100";
      slider.step = "1";
      slider.value = String(screen.brightness);
      slider.dataset.format = "percent100";
      slider.dataset.screen = screen.id;
      slider.setAttribute("aria-label", `Brilho de ${screen.name}`);
      slider.addEventListener("input", () => {
        showOutput(slider);
        setBrightness(screen.id, slider.valueAsNumber, slider).catch(showError);
      });
      row.append(name, slider, document.createElement("output"));
      showOutput(slider);
      return row;
    }),
  );
}

async function renderTweaks() {
  const tweaks = await invoke<WindowsTweaks>("get_windows_tweaks");
  document.querySelector<HTMLInputElement>("#tweak-transparency")!.checked =
    tweaks.transparency;
  const icons = document.querySelector<HTMLInputElement>("#tweak-icons")!;
  icons.checked = tweaks.desktopIcons ?? true;
  icons.disabled = tweaks.desktopIcons === null;

  const select = document.querySelector<HTMLSelectElement>("#tweak-cursor")!;
  const schemes = [...tweaks.cursorSchemes].sort((a, b) =>
    a.label.localeCompare(b.label, "pt-BR"),
  );
  const options = schemes.map((scheme) => new Option(scheme.label, scheme.id));
  if (tweaks.cursorScheme === null) {
    // A pointer set by hand or by another program matches no scheme.
    const custom = new Option("Personalizado", "");
    custom.disabled = true;
    options.unshift(custom);
  }
  select.replaceChildren(...options);
  select.value = tweaks.cursorScheme ?? "";
}

/** Runs a change in Windows, then shows what Windows actually did. */
async function changeTweak(command: string, args: Record<string, unknown>) {
  try {
    await invoke(command, args);
  } catch (error) {
    showError(error);
  }
  // Explorer applies the icon toggle a moment after being asked.
  await new Promise((resolve) => setTimeout(resolve, TWEAK_SETTLE_MS));
  await renderTweaks();
}

function showPage(name: string) {
  const known = document.querySelector(`.page[data-page="${name}"]`) !== null;
  const page = known ? name : DEFAULT_PAGE;
  for (const section of document.querySelectorAll<HTMLElement>(".page")) {
    section.hidden = section.dataset.page !== page;
    // The dock preview only means something on the dock's own pages.
    if (!section.hidden) stage.hidden = section.dataset.module !== "dock";
  }
  if (!stage.hidden) fitPreview();
  if (page === "general") {
    // Screens can be plugged in or out while the window is open, and Windows
    // settings may have changed since the last visit.
    renderScreens().catch(showError);
    renderTweaks().catch(showError);
  }
  for (const button of document.querySelectorAll<HTMLElement>("[data-nav]")) {
    if (button.dataset.nav === page) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }
  document.querySelector("#pages")!.scrollTop = 0;
  try {
    localStorage.setItem(PAGE_STORAGE_KEY, page);
  } catch {
    // Only a convenience: the window opens on the first page next time.
  }
}

function savedPage(): string {
  try {
    return localStorage.getItem(PAGE_STORAGE_KEY) ?? DEFAULT_PAGE;
  } catch {
    return DEFAULT_PAGE;
  }
}

function readControl(control: Control): string | number | boolean {
  if (control instanceof HTMLInputElement) {
    if (control.type === "checkbox") return control.checked;
    if (control.type === "range") return control.valueAsNumber;
  }
  return control.value;
}

function isCurrentTheme(theme: Theme): boolean {
  const keys = Object.keys(theme) as (keyof Theme)[];
  return keys.every((key) => theme[key] === config.theme[key]);
}

function themeCard(saved: SavedTheme, onRemove?: () => void): HTMLElement {
  const { theme, name } = saved;
  const card = document.createElement("div");
  card.className = "theme-card";
  card.classList.toggle("selected", isCurrentTheme(theme));

  const pick = document.createElement("button");
  pick.type = "button";
  pick.className = "pick";
  const scene = document.createElement("div");
  scene.className = "scene";
  const swatch = document.createElement("div");
  swatch.className = "swatch";
  swatch.style.background = barBackground(theme);
  swatch.style.borderColor = withAlpha(theme.borderColor, theme.borderOpacity);
  swatch.style.borderRadius = `${Math.min(theme.radius, 12)}px`;
  for (const color of [theme.dotColor, theme.activeColor, theme.labelColor]) {
    const dot = document.createElement("i");
    dot.style.background = color;
    swatch.append(dot);
  }
  const label = document.createElement("span");
  label.textContent = name;
  scene.append(swatch);
  pick.append(scene, label);
  pick.addEventListener("click", () => {
    config.theme = { ...theme };
    refreshControls();
    scheduleSave();
  });
  card.append(pick);

  if (onRemove) {
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "remove";
    remove.textContent = "×";
    remove.title = `Excluir ${name}`;
    remove.setAttribute("aria-label", `Excluir ${name}`);
    remove.addEventListener("click", onRemove);
    card.append(remove);
  }
  return card;
}

function renderThemes() {
  document
    .querySelector("#presets")!
    .replaceChildren(...PRESETS.map((preset) => themeCard(preset)));
  document.querySelector("#saved")!.replaceChildren(
    ...config.savedThemes.map((saved) =>
      themeCard(saved, () => {
        config.savedThemes = config.savedThemes.filter((item) => item !== saved);
        renderThemes();
        scheduleSave();
      }),
    ),
  );
  document.querySelector<HTMLElement>("#saved-empty")!.hidden =
    config.savedThemes.length > 0;
}

function appButton(
  text: string,
  label: string,
  disabled: boolean,
  action: () => void,
): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = text;
  button.title = label;
  button.disabled = disabled;
  button.setAttribute("aria-label", label);
  button.addEventListener("click", action);
  return button;
}

function moveApp(index: number, offset: number) {
  const [app] = config.apps.splice(index, 1);
  config.apps.splice(index + offset, 0, app);
  renderApps();
  scheduleSave();
}

function renderApps() {
  appsEl.replaceChildren(
    ...config.apps.map((app, index) => {
      const row = document.createElement("li");
      row.title = app.path;

      const name = document.createElement("input");
      name.type = "text";
      name.value = app.name;
      name.setAttribute("aria-label", `Nome de ${app.name}`);
      name.addEventListener("input", () => {
        app.name = name.value;
        scheduleSave();
      });

      row.append(
        iconImage(app.path),
        name,
        appButton("↑", `Mover ${app.name} para a esquerda`, index === 0, () =>
          moveApp(index, -1),
        ),
        appButton(
          "↓",
          `Mover ${app.name} para a direita`,
          index === config.apps.length - 1,
          () => moveApp(index, 1),
        ),
        appButton("×", `Remover ${app.name}`, false, () => {
          config.apps.splice(index, 1);
          renderApps();
          refreshRunning();
          scheduleSave();
        }),
      );
      return row;
    }),
  );
  document.querySelector<HTMLElement>("#apps-empty")!.hidden =
    config.apps.length > 0;
  renderPreview();
}

function moveWidget(index: number, offset: number) {
  const [widget] = config.widgets.splice(index, 1);
  config.widgets.splice(index + offset, 0, widget);
  renderWidgets();
  scheduleSave();
}

function renderWidgets() {
  document.querySelector("#widgets")!.replaceChildren(
    ...config.widgets.map((widget, index) => {
      const { name, description } = WIDGET_INFO[widget.id];
      const row = document.createElement("li");

      const label = document.createElement("label");
      const toggle = document.createElement("input");
      toggle.type = "checkbox";
      toggle.className = "switch";
      toggle.checked = widget.enabled;
      toggle.addEventListener("input", () => {
        widget.enabled = toggle.checked;
        renderPreview();
        scheduleSave();
      });
      const text = document.createElement("span");
      text.textContent = name;
      const hint = document.createElement("small");
      hint.textContent = description;
      text.append(hint);
      label.append(toggle, text);

      row.append(
        label,
        appButton("↑", `Mover ${name} para a esquerda`, index === 0, () =>
          moveWidget(index, -1),
        ),
        appButton(
          "↓",
          `Mover ${name} para a direita`,
          index === config.widgets.length - 1,
          () => moveWidget(index, 1),
        ),
      );
      return row;
    }),
  );
  renderPreview();
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf("\\") + 1);
}

function displayName(path: string): string {
  return fileName(path).replace(/\.[^.]+$/, "");
}

function addApp(path: string, name = displayName(path)) {
  config.apps.push({ name, path });
  renderApps();
  refreshRunning();
  scheduleSave();
}

async function refreshRunning() {
  const windows = await invoke<AppWindow[]>("get_windows");
  const pinned = new Set(
    config.apps.map((app) => fileName(app.path).toLowerCase()),
  );
  const seen = new Set<string>();
  const available = windows.filter((window) => {
    const key = fileName(window.exe).toLowerCase();
    if (pinned.has(key) || seen.has(key)) return false;
    seen.add(key);
    return true;
  });

  const current = Array.from(runningEl.children).map(
    (chip) => (chip as HTMLElement).title,
  );
  const next = available.map((window) => window.exe);
  if (current.join("|") !== next.join("|")) {
    runningEl.replaceChildren(
      ...available.map((window) => {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.title = window.exe;
        const label = document.createElement("span");
        label.textContent = displayName(window.exe);
        chip.append(iconImage(window.exe), label);
        chip.addEventListener("click", () => addApp(window.exe));
        return chip;
      }),
    );
  }
  document.querySelector<HTMLElement>("#running-empty")!.hidden =
    available.length > 0;
  runningExes = next;
  renderPreview();
}

function bindEvents() {
  form.addEventListener("submit", (event) => event.preventDefault());
  for (const control of controls()) {
    control.addEventListener("input", () => {
      target(control)[controlKey(control)] = readControl(control);
      showOutput(control);
      if (control.dataset.theme) renderThemes();
      refreshLook();
      scheduleSave();
    });
  }

  for (const button of document.querySelectorAll<HTMLElement>("[data-nav]")) {
    button.addEventListener("click", () => showPage(button.dataset.nav!));
  }
  window.addEventListener("resize", fitPreview);

  document.querySelector("#save-theme")!.addEventListener("click", () => {
    const name = themeName.value.trim();
    if (!name) {
      themeName.focus();
      return;
    }
    config.savedThemes = [
      ...config.savedThemes.filter((saved) => saved.name !== name),
      { name, theme: { ...config.theme } },
    ];
    themeName.value = "";
    renderThemes();
    scheduleSave();
  });

  document.querySelector("#add-file")!.addEventListener("click", async () => {
    const path = await open({
      title: "Escolher programa",
      filters: [
        { name: "Programas", extensions: ["exe"] },
        { name: "Todos os arquivos", extensions: ["*"] },
      ],
    });
    if (typeof path === "string") addApp(path);
  });

  document.querySelector("#reset")!.addEventListener("click", () => {
    config.theme = { ...PRESETS[0].theme };
    config.iconSize = 52;
    config.magnification = 1.6;
    config.magnifyStyle = "wave";
    config.edge = "bottom";
    config.alignment = "center";
    config.edgeGap = 8;
    config.customCss = "";
    refreshControls();
    scheduleSave();
  });

  document.querySelector("#choose-image")!.addEventListener("click", async () => {
    const path = await open({
      title: "Escolher imagem de fundo",
      filters: [{ name: "Imagens", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
    });
    if (typeof path !== "string") return;
    try {
      config.theme.barImage = await invoke<string>("load_bar_image", { path });
      renderThemes();
      refreshLook();
      scheduleSave();
    } catch (error) {
      showError(error);
    }
  });

  document.querySelector("#remove-image")!.addEventListener("click", () => {
    config.theme.barImage = "";
    renderThemes();
    refreshLook();
    scheduleSave();
  });

  const transparency = document.querySelector<HTMLInputElement>("#tweak-transparency")!;
  transparency.addEventListener("input", () =>
    changeTweak("set_transparency", { enabled: transparency.checked }),
  );
  const icons = document.querySelector<HTMLInputElement>("#tweak-icons")!;
  icons.addEventListener("input", () =>
    changeTweak("set_desktop_icons", { visible: icons.checked }),
  );
  const cursor = document.querySelector<HTMLSelectElement>("#tweak-cursor")!;
  cursor.addEventListener("input", () =>
    changeTweak("set_cursor_scheme", { id: cursor.value }),
  );

  document.querySelector("#full-brightness")!.addEventListener("click", async () => {
    for (const slider of document.querySelectorAll<HTMLInputElement>("[data-screen]")) {
      slider.value = "100";
      showOutput(slider);
      await setBrightness(slider.dataset.screen!, 100, slider).catch(showError);
    }
  });

  document.querySelector("#quit")!.addEventListener("click", () => {
    invoke("quit_app").catch(showError);
  });

  for (const id of ["#open-file", "#open-file-error"]) {
    document.querySelector(id)!.addEventListener("click", () => {
      invoke("open_config_file").catch(showError);
    });
  }
}

try {
  appConfig = await invoke<AppConfig>("get_config");
  config = appConfig.dock;
  bindEvents();
  showPage(savedPage());
  refreshControls();
  renderApps();
  renderWidgets();
  await refreshRunning();
  setInterval(() => refreshRunning().catch(showError), REFRESH_RUNNING_MS);

  // The tray can toggle the dock while this window is open.
  listen<AppConfig>("config-changed", ({ payload }) => {
    if (payload.dock.enabled !== config.enabled) {
      config.enabled = payload.dock.enabled;
      refreshControls();
    }
  });
  listen<string>("windows-theme-error", ({ payload }) => showError(payload));
  // Icons dragged on the dock must not be undone by the next save from here.
  listen<AppConfig>("dock-order-changed", ({ payload }) => {
    config.apps = payload.dock.apps;
    config.appOrder = payload.dock.appOrder;
    renderApps();
    refreshRunning().catch(showError);
  });
} catch (error) {
  // Saving now would overwrite the file the user needs to repair.
  form.hidden = true;
  showError(error);
  document
    .querySelector("#open-file-error")!
    .addEventListener("click", () => invoke("open_config_file"));
}
