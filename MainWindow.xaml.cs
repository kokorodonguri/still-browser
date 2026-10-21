using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.Wpf;
using System.Collections.ObjectModel;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Diagnostics;
using System.Windows.Input;
using System.Windows.Media;

namespace LightweightBrowser;

public partial class MainWindow : Window
{
    private readonly ObservableCollection<BrowserTab> tabs = new();
    private readonly List<string> bookmarks = new();
    private readonly List<HistoryEntry> history = new();
    private readonly List<SavedDownload> downloads = new();
    private readonly Dictionary<SavedDownload, TextBlock> downloadLabels = new();
    private Popup? libraryPopup;
    private readonly string dataFile = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "StillBrowser", "state.json");
    private BrowserTab? active;
    private bool isRestoring;
    private bool isClosing;
    private Task<CoreWebView2Environment>? environmentTask;
    private string? persistenceError;
    private bool backupUnreadableState;
    private readonly JsonSerializerOptions jsonOptions = new() { WriteIndented = true };
    // Thin capsule scrollbars for web pages. A cascade layer keeps any site's own
    // scrollbar styling in charge; forced-colors mode keeps the system scrollbars.
    private const string ScrollbarScript = """
        (() => {
          const sheet = new CSSStyleSheet();
          sheet.replaceSync(`@layer still-scrollbars { @media not (forced-colors: active) {
            ::-webkit-scrollbar { width: 10px; height: 10px; background: transparent; }
            ::-webkit-scrollbar-track, ::-webkit-scrollbar-corner { background: transparent; }
            ::-webkit-scrollbar-button { display: none; width: 0; height: 0; }
            ::-webkit-scrollbar-thumb { background: rgb(142 142 147 / 42%); background-clip: padding-box; border: 3px solid transparent; border-radius: 10px; min-height: 28px; }
            ::-webkit-scrollbar-thumb:hover { background-color: rgb(142 142 147 / 65%); border-width: 2px; }
            ::-webkit-scrollbar-thumb:active { background-color: rgb(142 142 147 / 80%); border-width: 2px; }
          } }`);
          document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
        })();
        """;

    public MainWindow()
    {
        InitializeComponent();
        PreviewKeyDown += (_, e) => {
            if (IsBrowserShortcut(e.Key, Keyboard.Modifiers)) {
                // WPF WebView2 forwards native accelerators as PreviewKeyDown.
                // Defer browser calls until its synchronous input callback returns.
                var key = e.Key; var modifiers = Keyboard.Modifiers;
                e.Handled = true; Dispatcher.BeginInvoke(new Action(() => RunShortcut(key, modifiers)));
            }
        };
        Activated += (_, _) => { if (noticePending && !isClosing) NoticePopup.IsOpen = true; };
        Deactivated += (_, _) => NoticePopup.IsOpen = false;
        Loaded += async (_, _) => await RestoreAsync();
        Closing += (_, _) => { NoticePopup.IsOpen = false; if (libraryPopup != null) libraryPopup.IsOpen = false; SaveState(); isClosing = true; foreach (var tab in tabs) { tab.Closed = true; tab.View?.Dispose(); } };
    }

    private async Task RestoreAsync()
    {
        isRestoring = true;
        RootLayout.IsEnabled = false;
        StatusMessage = "Restoring your tabs…";
        BrowserTab? selected = null;
        BrowserState? state = null;
        try { if (File.Exists(dataFile)) state = JsonSerializer.Deserialize<BrowserState>(await File.ReadAllTextAsync(dataFile)); }
        catch (JsonException) { backupUnreadableState = true; persistenceError = "Saved browser data could not be read. A new tab will open; the old data will be backed up."; }
        catch (IOException) { persistenceError = "Saved browser data could not be read. Check folder permissions."; }
        catch (UnauthorizedAccessException) { persistenceError = "Saved browser data could not be read. Check folder permissions."; }
        if (state != null)
        {
            bookmarks.AddRange((state.Bookmarks ?? []).Where(x => !string.IsNullOrWhiteSpace(x)).Distinct());
            history.AddRange((state.History ?? []).Where(x => x != null && !string.IsNullOrWhiteSpace(x.Url)).TakeLast(500));
            downloads.AddRange((state.Downloads ?? []).Where(x => x != null && !string.IsNullOrWhiteSpace(x.Path) && !x.Path.Contains('\0')));
            foreach (var download in downloads.Where(x => x.State == "InProgress")) download.State = "Interrupted";
            if (state.Tabs?.Count > 0)
            {
                foreach (var item in state.Tabs.Where(x => x != null)) AddTab(item.Url, false);
                if (tabs.Count > 0)
                {
                    var index = Math.Clamp(state.ActiveIndex, 0, tabs.Count - 1);
                    selected = tabs[index];
                }
            }
        }
        SetSidebar(state?.SidebarOpen ?? false);
        isRestoring = false;
        RootLayout.IsEnabled = true;
        if (isClosing) return;
        if (tabs.Count == 0) AddTab("about:blank", true);
        else if (selected != null) SelectTab(selected);
    }

    private void AddTab(string url = "about:blank", bool select = true)
    {
        url = url == "about:blank" ? url : Normalize(url);
        var tab = new BrowserTab { Url = url, Title = url == "about:blank" ? "New tab" : GetHost(url) };
        tabs.Add(tab);
        var row = new Grid { Height = 42, Margin = new Thickness(0, 1, 0, 1), Tag = tab };
        row.ColumnDefinitions.Add(new ColumnDefinition()); row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(32) });
        var selectButton = new Button { Padding = new Thickness(10, 5, 6, 5), HorizontalContentAlignment = HorizontalAlignment.Left, Margin = new Thickness(0), Tag = tab };
        selectButton.Click += (_, _) => SelectTab(tab);
        var label = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        var title = new TextBlock { Text = tab.Title, FontSize = 12, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, MaxWidth = 150 };
        label.Children.Add(title); tab.TitleLabel = title; selectButton.Content = label;
        Grid.SetColumn(selectButton, 0); row.Children.Add(selectButton);
        var close = new Button { Content = "×", ToolTip = "Close tab", Padding = new Thickness(4), FontSize = 15, Visibility = Visibility.Collapsed, Tag = tab };
        close.Click += (_, _) => CloseTab(tab); Grid.SetColumn(close, 1); row.Children.Add(close);
        row.MouseEnter += (_, _) => close.Visibility = Visibility.Visible;
        row.MouseLeave += (_, _) => close.Visibility = active == tab || row.IsKeyboardFocusWithin ? Visibility.Visible : Visibility.Collapsed;
        row.IsKeyboardFocusWithinChanged += (_, _) => close.Visibility = active == tab || row.IsKeyboardFocusWithin ? Visibility.Visible : Visibility.Collapsed;
        tab.ListRow = row; TabList.Children.Add(row);
        if (select) { SelectTab(tab); if (url == "about:blank") FocusAddress(); }
    }

    private void SelectTab(BrowserTab tab)
    {
        if (!tabs.Contains(tab)) return;
        if (libraryPopup != null) libraryPopup.IsOpen = false;
        active = tab;
        foreach (var item in tabs)
        {
            if (item.ListRow != null) { item.ListRow.Background = item == tab ? new SolidColorBrush(Color.FromRgb(44, 44, 46)) : Brushes.Transparent; ((Button)item.ListRow.Children[1]).Visibility = item == tab || item.ListRow.IsKeyboardFocusWithin ? Visibility.Visible : Visibility.Collapsed; }
        }
        foreach (var item in tabs) if (item.TitleLabel != null) item.TitleLabel.FontWeight = item == tab ? FontWeights.SemiBold : FontWeights.Normal;
        ContentHost.Children.Clear();
        if (tab.View != null) ContentHost.Children.Add(tab.View);
        else if (tab.Url == "about:blank") ContentHost.Children.Add(CreateHome());
        AddressBox.Text = tab.Url == "about:blank" ? "" : tab.Url;
        Title = tab.Url == "about:blank" ? "Still" : tab.Title + " — Still";
        BookmarkButton.IsEnabled = tab.Url != "about:blank";
        StatusMessage = persistenceError ?? (tab.Loading ? "Loading…" : "Ready");
        BackButton.IsEnabled = tab.View?.CanGoBack == true; ForwardButton.IsEnabled = tab.View?.CanGoForward == true;
        ReloadButton.IsEnabled = tab.View?.CoreWebView2 != null || tab.Initialization?.IsCompleted != false;
        if (!isRestoring && tab.Url != "about:blank" && tab.View == null) _ = NavigateAsync(tab, tab.Url);
    }

    private UIElement CreateHome() => new Grid { Background = (Brush)FindResource("CanvasBrush") };

    private bool sidebarOpen;
    private bool noticePending;
    private string StatusMessage
    {
        set
        {
            LoadingIndicator.Visibility = active?.Loading == true ? Visibility.Visible : Visibility.Collapsed;
            if (value is "Ready" or "Loading…" or "Restoring your tabs…") return;
            noticePending = true;
            NoticeText.Text = value;
            NoticePopup.IsOpen = !isClosing;
        }
    }

    private void SetSidebar(bool open)
    {
        if (!open && SidebarPanel.IsKeyboardFocusWithin) SidebarButton.Focus();
        sidebarOpen = open;
        SidebarColumn.Width = new GridLength(open ? 224 : 0);
        SidebarPanel.Visibility = open ? Visibility.Visible : Visibility.Collapsed;
        SidebarButton.ToolTip = open ? "Hide tabs" : "Show tabs";
        System.Windows.Automation.AutomationProperties.SetName(SidebarButton, open ? "Hide tabs" : "Show tabs");
    }
    private void Sidebar_Click(object sender, RoutedEventArgs e) { SetSidebar(!sidebarOpen); SaveState(); }
    private void Menu_Click(object sender, RoutedEventArgs e)
    {
        noticePending = false;
        NoticePopup.IsOpen = false;
        if (MenuButton.ContextMenu is { } menu) { menu.PlacementTarget = MenuButton; menu.Placement = PlacementMode.Bottom; menu.IsOpen = true; }
    }
    private void DismissNotice_Click(object sender, RoutedEventArgs e) { noticePending = false; NoticePopup.IsOpen = false; MenuButton.Focus(); }
    private void FocusAddress() { AddressBox.Focus(); AddressBox.SelectAll(); }
    private static bool IsBrowserShortcut(Key key, ModifierKeys modifiers) =>
        modifiers == ModifierKeys.Control && key is Key.L or Key.T or Key.W or Key.Tab ||
        modifiers == (ModifierKeys.Control | ModifierKeys.Shift) && key == Key.Tab;
    private void RunShortcut(Key key, ModifierKeys modifiers)
    {
        if (isClosing || isRestoring) return;
        switch (key)
        {
            case Key.L: FocusAddress(); break;
            case Key.T: AddTab(); SaveState(); break;
            case Key.W: if (active != null) CloseTab(active); break;
            case Key.Tab:
                if (active != null && tabs.Count > 0)
                    SelectTab(tabs[(tabs.IndexOf(active) + (modifiers.HasFlag(ModifierKeys.Shift) ? -1 : 1) + tabs.Count) % tabs.Count]);
                break;
        }
    }
    private void UpdateDownloadsIndicator()
    {
        var running = downloads.Where(d => d.State == "InProgress").ToList();
        ActiveDownloadsButton.Visibility = running.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        ActiveDownloadsButton.Content = $"↓ {running.Count}";
        ActiveDownloadsButton.ToolTip = string.Join("\n", running.Select(DescribeDownload));
        System.Windows.Automation.AutomationProperties.SetName(ActiveDownloadsButton, $"{running.Count} downloads in progress");
    }

    private Task<WebView2> EnsureViewAsync(BrowserTab tab)
    {
        if (tab.Initialization?.IsFaulted == true || tab.Initialization?.IsCanceled == true) tab.Initialization = null;
        return tab.Initialization ??= InitializeViewAsync(tab);
    }

    private async Task<WebView2> InitializeViewAsync(BrowserTab tab)
    {
        var view = new WebView2 { DefaultBackgroundColor = System.Drawing.Color.FromArgb(28, 28, 30) };
        tab.View = view;
        if (active == tab) { ContentHost.Children.Clear(); ContentHost.Children.Add(view); ReloadButton.IsEnabled = false; }
        try
        {
            environmentTask ??= CoreWebView2Environment.CreateAsync(userDataFolder: Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "StillBrowser", "WebView2"));
            await view.EnsureCoreWebView2Async(await environmentTask);
            if (tab.Closed || isClosing) { view.Dispose(); throw new OperationCanceledException(); }
            tab.View = view;
            if (active == tab) ReloadButton.IsEnabled = true;
        }
        catch
        {
            view.Dispose(); tab.Initialization = null;
            if (tab.View == view) tab.View = null;
            if (active == tab && !isClosing) ReloadButton.IsEnabled = true;
            if (environmentTask?.IsFaulted == true) environmentTask = null;
            throw;
        }
        view.CoreWebView2.Settings.AreDefaultContextMenusEnabled = true;
        view.CoreWebView2.Settings.IsStatusBarEnabled = false;
        await view.CoreWebView2.AddScriptToExecuteOnDocumentCreatedAsync(ScrollbarScript);
        view.CoreWebView2.NewWindowRequested += (_, e) => { e.Handled = true; if (!tab.Closed && !isClosing) AddTab(e.Uri, true); };
        view.CoreWebView2.NavigationStarting += (_, _) => { tab.Loading = true; if (active == tab) StatusMessage = "Loading…"; };
        view.CoreWebView2.SourceChanged += (_, _) =>
        {
            if (tab.Closed) return;
            tab.Url = view.CoreWebView2.Source;
            if (active == tab) BookmarkButton.IsEnabled = tab.Url != "about:blank";
            if (active == tab && !AddressBox.IsKeyboardFocusWithin) AddressBox.Text = tab.Url == "about:blank" ? "" : tab.Url;
            SaveState();
        };
        view.CoreWebView2.HistoryChanged += (_, _) => { if (active == tab) { BackButton.IsEnabled = view.CanGoBack; ForwardButton.IsEnabled = view.CanGoForward; } };
        view.CoreWebView2.NavigationCompleted += (_, e) =>
        {
            if (tab.Closed) return;
            tab.Loading = false;
            if (e.IsSuccess && tab.Url != "about:blank")
            {
                history.Add(new HistoryEntry(tab.Url, DateTimeOffset.Now, tab.Title));
                if (history.Count > 500) history.RemoveAt(0);
                SaveState();
            }
            if (active == tab) { if (!AddressBox.IsKeyboardFocusWithin) AddressBox.Text = tab.Url == "about:blank" ? "" : tab.Url; BackButton.IsEnabled = view.CanGoBack; ForwardButton.IsEnabled = view.CanGoForward; StatusMessage = persistenceError ?? (e.IsSuccess ? "Ready" : "This page could not load. Check your connection and reload to try again."); }
        };
        view.CoreWebView2.DocumentTitleChanged += (_, _) => Dispatcher.Invoke(() => { if (tab.Closed || isClosing) return; var title = view.CoreWebView2.DocumentTitle; tab.Title = string.IsNullOrWhiteSpace(title) ? GetHost(tab.Url) : title; if (tab.TitleLabel != null) tab.TitleLabel.Text = tab.Title; if (active == tab) Title = tab.Url == "about:blank" ? "Still" : tab.Title + " — Still"; });
        view.CoreWebView2.DownloadStarting += (_, e) =>
        {
            var operation = e.DownloadOperation;
            var download = new SavedDownload { Path = operation.ResultFilePath, State = "InProgress" };
            downloads.Add(download);
            void Update()
            {
                if (isClosing) return;
                download.Path = operation.ResultFilePath; download.Bytes = operation.BytesReceived; download.Total = (long)Math.Min(operation.TotalBytesToReceive ?? 0, (ulong)long.MaxValue); download.State = operation.State.ToString();
                if (downloadLabels.TryGetValue(download, out var label)) label.Text = DescribeDownload(download);
                UpdateDownloadsIndicator();
                if (operation.State != CoreWebView2DownloadState.InProgress) { SaveState(); StatusMessage = $"Download {download.State.ToLowerInvariant()}: {Path.GetFileName(download.Path)}"; }
            }
            operation.BytesReceivedChanged += (_, _) => Update();
            operation.StateChanged += (_, _) => Update();
            UpdateDownloadsIndicator();
            SaveState();
        };
        return view;
    }

    private async Task NavigateAsync(BrowserTab tab, string value)
    {
        var url = Normalize(value);
        if (url == "about:blank" || tab.Closed || isClosing) return;
        if (!Uri.TryCreate(url, UriKind.Absolute, out var address))
        {
            if (active == tab) StatusMessage = "The address or search text is too long or invalid. Shorten it and try again.";
            return;
        }
        var navigation = ++tab.Navigation;
        tab.Url = url;
        tab.Loading = true;
        if (active == tab) StatusMessage = "Loading…";
        try
        {
            var view = await EnsureViewAsync(tab);
            if (tab.Closed || isClosing || tab.Navigation != navigation) return;
            view.Source = address;
            if (active == tab) { ContentHost.Children.Clear(); ContentHost.Children.Add(view); AddressBox.Text = url; }
            SaveState();
        }
        catch (OperationCanceledException) { }
        catch (WebView2RuntimeNotFoundException)
        {
            tab.Loading = false;
            if (active == tab && !tab.Closed && !isClosing) StatusMessage = "The page could not open. Install Microsoft Edge WebView2 Runtime, then try again.";
        }
        catch (Exception error) when (error is InvalidOperationException or System.Runtime.InteropServices.COMException or ArgumentException)
        {
            tab.Loading = false;
            if (active == tab && !tab.Closed && !isClosing) StatusMessage = "The page could not open. Reload to try again, or open it in a new tab.";
        }
    }

    private static string Normalize(string input)
    {
        var text = (input ?? "").Trim(); if (string.IsNullOrWhiteSpace(text)) return "about:blank";
        if (Uri.TryCreate(text, UriKind.Absolute, out var uri) && (uri.Scheme == "http" || uri.Scheme == "https")) return uri.ToString();
        if (!text.Contains(' ') && text.Contains('.') && Uri.TryCreate("https://" + text, UriKind.Absolute, out var host)) return host.ToString();
        return "https://www.google.com/search?q=" + Uri.EscapeDataString(text);
    }
    private static string GetHost(string url) => Uri.TryCreate(url, UriKind.Absolute, out var uri) && !string.IsNullOrEmpty(uri.Host) ? uri.Host : "New tab";
    private void NewTab_Click(object sender, RoutedEventArgs e) { AddTab(); SaveState(); }
    private async void AddressBox_KeyDown(object sender, KeyEventArgs e) { if (e.Key == Key.Enter && active != null) { e.Handled = true; var input = sender is TextBox box ? box.Text : AddressBox.Text; if (string.IsNullOrWhiteSpace(input)) { AddressBox.Text = active.Url == "about:blank" ? "" : active.Url; return; } AddressBox.Text = input; var target = active; var navigation = target.Navigation + 1; await NavigateAsync(target, input); if (active == target && target.Navigation == navigation) Keyboard.ClearFocus(); } }
    private void Back_Click(object sender, RoutedEventArgs e) { if (active?.View?.CanGoBack == true) active.View.GoBack(); }
    private void Forward_Click(object sender, RoutedEventArgs e) { if (active?.View?.CanGoForward == true) active.View.GoForward(); }
    private void Reload_Click(object sender, RoutedEventArgs e) { if (active?.View?.CoreWebView2 != null) active.View.Reload(); else if (active?.Initialization?.IsCompleted == false) StatusMessage = "The browser is starting. Please wait before reloading."; else if (active != null) SelectTab(active); }
    private void CloseTab(BrowserTab tab) { var idx = tabs.IndexOf(tab); if (idx < 0) return; tab.Closed = true; tabs.Remove(tab); TabList.Children.Remove(tab.ListRow); tab.View?.Dispose(); if (tabs.Count == 0) AddTab(); else if (active == tab) SelectTab(tabs[Math.Min(idx, tabs.Count - 1)]); SaveState(); }
    private void BookmarkCurrent_Click(object sender, RoutedEventArgs e)
    {
        if (active == null || active.Url == "about:blank") { StatusMessage = "Open a page before adding a bookmark."; return; }
        var url = active.Url;
        if (bookmarks.Contains(url)) { StatusMessage = "This page is already bookmarked."; return; }
        bookmarks.Add(url);
        if (SaveState()) StatusMessage = "Added to bookmarks";
        else bookmarks.Remove(url);
    }
    private void Bookmarks_Click(object sender, RoutedEventArgs e) => ShowItems("Bookmarks", bookmarks, "No bookmarks yet. Star a page to save it.");
    private void History_Click(object sender, RoutedEventArgs e) => ShowItems("Recently visited", history.AsEnumerable().Reverse().Select(x => x.Url).Distinct().Take(30).ToList(), "Your browsing history will appear here.");
    private void Downloads_Click(object sender, RoutedEventArgs e)
    {
        downloadLabels.Clear();
        var rows = new List<UIElement>();
        foreach (var download in downloads.AsEnumerable().Reverse())
        {
            var label = new TextBlock { Text = DescribeDownload(download), TextWrapping = TextWrapping.Wrap };
            downloadLabels[download] = label;
            var row = new Button { Content = label, HorizontalContentAlignment = HorizontalAlignment.Left, ToolTip = "Show the download folder", Padding = new Thickness(8, 7, 8, 7) };
            row.Click += (_, _) =>
            {
                var directory = Path.GetDirectoryName(download.Path);
                if (string.IsNullOrWhiteSpace(directory) || !Directory.Exists(directory)) { StatusMessage = "The download folder is no longer available."; return; }
                try { Process.Start(new ProcessStartInfo(directory) { UseShellExecute = true }); }
                catch (System.ComponentModel.Win32Exception) { StatusMessage = "The download folder could not open. Try opening it in File Explorer."; }
            };
            rows.Add(row);
        }
        ShowPanel("Downloads", rows, "No downloads yet. Download a file from a page to see its progress and save location.");
    }
    private static string DescribeDownload(SavedDownload download) => $"{Path.GetFileName(download.Path)}\n{download.State} · {download.Bytes / 1024} KB" + (download.Total > 0 ? $" / {download.Total / 1024} KB" : "") + $"\n{download.Path}";
    private void ShowItems(string title, IList<string> items, string empty)
    {
        var rows = new List<UIElement>();
        foreach (var item in items) { var b = new Button { Content = new TextBlock { Text = item, TextWrapping = TextWrapping.Wrap }, HorizontalContentAlignment = HorizontalAlignment.Left, Padding = new Thickness(8, 7, 8, 7), Tag = item }; b.Click += async (_, _) => { if (active != null) { if (libraryPopup != null) libraryPopup.IsOpen = false; await NavigateAsync(active, item); } }; rows.Add(b); }
        ShowPanel(title, rows, empty);
    }
    private void ShowPanel(string title, IList<UIElement> rows, string empty)
    {
        noticePending = false;
        NoticePopup.IsOpen = false;
        if (libraryPopup != null) libraryPopup.IsOpen = false;
        var panel = new Border { Width = Math.Min(420, Math.Max(260, ContentHost.ActualWidth - 44)), Background = new SolidColorBrush(Color.FromRgb(44, 44, 46)), BorderBrush = new SolidColorBrush(Color.FromRgb(64, 64, 66)), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(16) };
        System.Windows.Documents.TextElement.SetForeground(panel, new SolidColorBrush(Color.FromRgb(245, 245, 247)));
        KeyboardNavigation.SetTabNavigation(panel, KeyboardNavigationMode.Cycle);
        panel.PreviewKeyDown += (_, e) => { if (e.Key == Key.Escape && libraryPopup != null) { libraryPopup.IsOpen = false; e.Handled = true; } };
        var stack = new StackPanel(); var header = new DockPanel { Margin = new Thickness(0, 0, 0, 12) }; var close = new Button { Content = "×", ToolTip = "Close" }; DockPanel.SetDock(close, Dock.Right); close.Click += (_, _) => { if (libraryPopup != null) libraryPopup.IsOpen = false; }; header.Children.Add(close); header.Children.Add(new TextBlock { Text = title, FontSize = 16, FontWeight = FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center }); stack.Children.Add(header);
        var items = new StackPanel();
        if (rows.Count == 0) items.Children.Add(new TextBlock { Text = empty, Foreground = new SolidColorBrush(Color.FromRgb(174, 174, 178)), Margin = new Thickness(0, 6, 0, 10), TextWrapping = TextWrapping.Wrap });
        else foreach (var row in rows) items.Children.Add(row);
        stack.Children.Add(new ScrollViewer { Content = items, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, MaxHeight = Math.Max(140, Math.Min(400, ContentHost.ActualHeight - 100)) });
        panel.Child = stack;
        // A Popup owns a separate HWND, so WebView2's native surface cannot cover it.
        libraryPopup = new Popup { Child = panel, PlacementTarget = ContentHost, Placement = PlacementMode.Center, StaysOpen = false, AllowsTransparency = true };
        libraryPopup.Opened += (_, _) => close.Focus();
        libraryPopup.Closed += (_, _) => { if (!isClosing && IsActive) MenuButton.Focus(); };
        libraryPopup.IsOpen = true;
    }
    private bool SaveState()
    {
        if (isRestoring) return false;
        try { Directory.CreateDirectory(Path.GetDirectoryName(dataFile)!); if (backupUnreadableState) { File.Copy(dataFile, dataFile + ".invalid-" + DateTime.UtcNow.ToString("yyyyMMddHHmmss")); backupUnreadableState = false; } var state = new BrowserState { Tabs = tabs.Select(x => new SavedTab(x.Url)).ToList(), ActiveIndex = active == null ? 0 : Math.Max(0, tabs.IndexOf(active)), Bookmarks = bookmarks, History = history, Downloads = downloads, SidebarOpen = sidebarOpen }; var temporary = dataFile + ".tmp"; File.WriteAllText(temporary, JsonSerializer.Serialize(state, jsonOptions)); File.Move(temporary, dataFile, true); persistenceError = null; return true; }
        catch (IOException) { persistenceError = StatusMessage = "Changes could not be saved. Check available disk space and folder permissions."; }
        catch (UnauthorizedAccessException) { persistenceError = StatusMessage = "Changes could not be saved. Check folder permissions."; }
        return false;
    }
}

public sealed class BrowserTab { public string Url { get; set; } = "about:blank"; public string Title { get; set; } = "New tab"; public WebView2? View { get; set; } public Task<WebView2>? Initialization { get; set; } public bool Closed { get; set; } public bool Loading { get; set; } public int Navigation { get; set; } public Grid? ListRow { get; set; } public TextBlock? TitleLabel { get; set; } }
public sealed record SavedTab(string Url);
public sealed record HistoryEntry(string Url, DateTimeOffset VisitedAt, string Title);
public sealed class SavedDownload { public string Path { get; set; } = ""; public string State { get; set; } = ""; public long Bytes { get; set; } public long Total { get; set; } }
public sealed class BrowserState { public bool SidebarOpen { get; set; } public List<SavedTab>? Tabs { get; set; } public int ActiveIndex { get; set; } public List<string>? Bookmarks { get; set; } public List<HistoryEntry>? History { get; set; } public List<SavedDownload>? Downloads { get; set; } }
