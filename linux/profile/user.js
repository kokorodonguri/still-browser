// Still defaults. Browser data belongs to this profile, not the system Firefox.
user_pref("toolkit.legacyUserProfileCustomizations.stylesheets", true);
user_pref("sidebar.revamp", true);
user_pref("sidebar.verticalTabs", true);
user_pref("sidebar.visibility", "hide-sidebar");
user_pref("sidebar.main.tools", "");
user_pref("browser.tabs.inTitlebar", 1);
user_pref("browser.uidensity", 0);
user_pref("browser.theme.toolbar-theme", 0);
user_pref("browser.theme.content-theme", 0);
user_pref("ui.systemUsesDarkTheme", 1);
user_pref("browser.startup.page", 3);
user_pref("browser.sessionstore.restore_on_demand", true);
user_pref("browser.sessionstore.restore_pinned_tabs_on_demand", true);
user_pref("browser.startup.homepage", "about:home");
user_pref("browser.newtabpage.enabled", true);
user_pref("browser.newtabpage.activity-stream.showSearch", false);
// Keep typing in the visible home input instead of transferring hidden focus.
user_pref("browser.newtabpage.activity-stream.improvesearch.handoffToAwesomebar", false);
user_pref("browser.newtabpage.activity-stream.feeds.section.topstories", false);
user_pref("browser.newtabpage.activity-stream.feeds.section.highlights", false);
user_pref("browser.newtabpage.activity-stream.showSponsored", false);
user_pref("browser.newtabpage.activity-stream.showSponsoredTopSites", false);
user_pref("browser.newtabpage.activity-stream.default.sites", "https://www.google.com/,https://www.youtube.com/,https://www.wikipedia.org/,https://github.com/");
// Use these local defaults immediately instead of waiting on a remote tile feed.
user_pref("browser.topsites.useRemoteSetting", false);
user_pref("browser.newtabpage.activity-stream.topSitesRows", 1);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.firstrunSkipsHomepage", true);
user_pref("browser.download.alwaysOpenPanel", true);
user_pref("browser.download.useDownloadDir", true);
user_pref("browser.download.folderList", 1);
// Address bar: the search engine's suggestions next to history and bookmarks.
user_pref("browser.urlbar.suggest.searches", true);
user_pref("browser.search.suggest.enabled", true);
user_pref("still.suggestRevision", 1);
