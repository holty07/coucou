// Coucou for KDE Plasma.
//
// The app (the Rust/Tauri `coucou` binary) runs in the background and owns
// everything that matters: the Claude Code relay, the keys, the integrations.
// This widget is only the island's window. It renders the very same front end
// in a QtWebEngine view, served by the app on 127.0.0.1 and driven over a
// token-protected WebSocket (see windows/src-tauri/src/plasma.rs).
//
// In a panel it shows a small Mochi, and the island opens in its popup. On the
// desktop it is a board instead: every Claude Code session, the plan's usage and
// the integrations, without ever acting or opening anything on its own.

import QtQuick
import QtQuick.Layouts
import QtWebEngine
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as P5Support
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root

    readonly property bool inPanel: Plasmoid.formFactor === PlasmaCore.Types.Horizontal
        || Plasmoid.formFactor === PlasmaCore.Types.Vertical

    /** Where the page and the socket are, from $XDG_RUNTIME_DIR/coucou/plasma.json. */
    property var conn: null
    property bool connected: false

    // Mirrored from the page, for the panel Mochi and the popup size.
    property string botState: "idle"
    property color botColor: "#FFFFFF"
    property string islandMode: "hidden"
    property int islandHeight: 160
    property bool paused: false
    /** Every Mochi that wants you, most urgent first: {id, name, color, state}. */
    property var attention: []
    /** Desktop: what the board shows, worked out by board.html. */
    property var board: null

    function wantsText(a) {
        switch (a.state) {
        case "approval": return i18n("%1 needs your approval", a.name);
        case "question": return a.id === "integration_slack" ? i18n("%1: unread DMs", a.name)
            : a.id === "integration_github" ? i18n("%1: pull requests waiting on you", a.name)
            : i18n("%1 has a question", a.name);
        case "error": return i18n("%1 hit an error", a.name);
        case "finished": return i18n("%1 is ready", a.name);
        }
        return a.name;
    }

    readonly property string pageUrl: conn
        ? conn.page + (inPanel ? "/index.html?host=panel" : "/board.html?host=desktop")
            + "#ws=" + encodeURIComponent(conn.ws) + "&token=" + conn.token
        : ""

    preferredRepresentation: inPanel ? compactRepresentation : fullRepresentation
    // The page must be alive even while the popup is shut: it is what hears
    // Claude Code and decides when to open.
    preloadFullRepresentation: true
    Plasmoid.backgroundHints: inPanel
        ? PlasmaCore.Types.DefaultBackground
        : PlasmaCore.Types.NoBackground | PlasmaCore.Types.ConfigurableBackground

    toolTipMainText: "Coucou"
    toolTipSubText: !connected ? i18n("Coucou isn't running")
        : paused ? i18n("Paused")
        : attention.length > 0 ? attention.map(wantsText).join("\n")
        : botState === "idle" ? i18n("Watching Claude Code") : botState

    Plasmoid.contextualActions: [
        PlasmaCore.Action {
            text: i18n("Coucou Settings…")
            icon.name: "configure"
            enabled: root.connected
            onTriggered: root.sendCommand("open_settings_window")
        },
        PlasmaCore.Action {
            text: root.connected ? i18n("Reload") : i18n("Start Coucou")
            icon.name: root.connected ? "view-refresh" : "media-playback-start"
            onTriggered: root.connected ? root.reload() : root.startApp()
        }
    ]

    // ── Finding the app ─────────────────────────────────────────────────────

    P5Support.DataSource {
        id: exec
        engine: "executable"
        connectedSources: []
        onNewData: (source, data) => {
            disconnectSource(source);
            if (source === root.readCmd) root.gotConnection(data["stdout"] || "");
        }
        function run(cmd) {
            connectSource(cmd);
        }
    }

    // Prints the connection file only while the process that wrote it is alive,
    // so a stale file from a crashed app reads as "not running".
    readonly property string readCmd: "f=\"${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/coucou/plasma.json\"; "
        + "p=$(grep -o '\"pid\":[0-9]*' \"$f\" 2>/dev/null | cut -d: -f2); "
        + "[ -n \"$p\" ] && kill -0 \"$p\" 2>/dev/null && cat \"$f\""

    function gotConnection(text) {
        let next = null;
        try {
            next = text.trim() ? JSON.parse(text) : null;
        } catch (e) {
            next = null;
        }
        if (!next || !next.page || !next.ws || !next.token) {
            if (conn) conn = null;
            connected = false;
            return;
        }
        if (!conn || conn.token !== next.token || conn.page !== next.page) {
            connected = false;
            conn = next;
        }
    }

    Timer {
        // Fast while we wait for the app, slow once it is there (just to notice
        // it went away without a goodbye).
        interval: root.connected ? 10000 : 2000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: exec.run(root.readCmd)
    }

    function startApp() {
        exec.run("command -v coucou >/dev/null 2>&1 && setsid -f coucou >/dev/null 2>&1 "
            + "|| setsid -f \"$HOME/.local/bin/coucou\" >/dev/null 2>&1");
    }

    function reload() {
        conn = null;
        connected = false;
        exec.run(readCmd);
    }

    // ── Page ⇄ widget ───────────────────────────────────────────────────────

    property var web: null

    function toPage(msg) {
        if (web && connected)
            web.runJavaScript("window.coucouHost && window.coucouHost.receive(" + JSON.stringify(msg) + ")");
    }

    /** A command the widget itself issues (the context menu), sent through the page. */
    function sendCommand(cmd) {
        if (web && connected)
            web.runJavaScript("window.coucouHost && window.coucouHost.receive("
                + JSON.stringify({ type: "command", payload: cmd }) + ")");
    }

    function fromPage(msg) {
        switch (msg.type) {
        case "connected":
            connected = true;
            break;
        case "disconnected":
            connected = false;
            conn = null;
            break;
        case "expand":
            if (inPanel) root.expanded = true;
            break;
        case "collapse":
            if (inPanel) root.expanded = false;
            break;
        case "board":
            board = msg.payload;
            break;
        case "state":
            botState = msg.payload.bot;
            botColor = msg.payload.color;
            islandMode = msg.payload.mode;
            paused = msg.payload.paused;
            attention = msg.payload.attention || [];
            tasks = msg.payload.tasks || [];
            if (msg.payload.size && msg.payload.size.h > 0) islandHeight = msg.payload.size.h;
            break;
        }
    }

    onExpandedChanged: function () {
        if (inPanel) toPage({ type: root.expanded ? "open" : "close" });
    }

    // ── Panel: the small Mochi ──────────────────────────────────────────────

    /** Every pill, in the island's order: {id, name, color, state, glow}. */
    property var tasks: []

    compactRepresentation: Item {
        id: compact
        readonly property bool vertical: Plasmoid.formFactor === PlasmaCore.Types.Vertical
        // One square per Mochi, side by side with no gap — the island's compact
        // bar without the notch.
        readonly property real cell: vertical ? width : height
        readonly property int count: root.connected ? Math.max(1, root.tasks.length) : 1
        Layout.minimumWidth: vertical ? -1 : cell * count
        Layout.preferredWidth: vertical ? -1 : cell * count
        Layout.minimumHeight: vertical ? cell * count : -1
        Layout.preferredHeight: vertical ? cell * count : -1

        Grid {
            anchors.centerIn: parent
            columns: compact.vertical ? 1 : compact.count
            spacing: 0

            Repeater {
                // Not running: one sleeping Mochi that starts the app on a click.
                model: root.connected && root.tasks.length > 0
                    ? root.tasks
                    : [{ id: "", name: "Coucou", color: "", state: "sleeping", glow: "#FFFFFF" }]

                delegate: MouseArea {
                    id: slot
                    required property var modelData
                    width: compact.cell
                    height: compact.cell
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton
                    onClicked: {
                        if (!root.connected) return root.startApp();
                        if (root.expanded) return root.expanded = false;
                        if (modelData.id) root.toPage({ type: "focus", payload: modelData.id });
                        root.expanded = true;
                    }

                    PanelMochi {
                        anchors.centerIn: parent
                        width: parent.width * 0.96
                        height: width
                        mood: slot.modelData.state
                        glow: slot.modelData.glow
                        // Claude Code is the grey Mochi, as on the island; the
                        // integrations wear their pill colour.
                        body: slot.modelData.id && slot.modelData.id !== "integration_claude"
                            ? slot.modelData.color : "transparent"
                        awake: root.connected && !root.paused
                        hovered: slot.containsMouse
                    }

                }
            }
        }

        // Dragging a file onto the panel opens the island to drop it into.
        DropArea {
            anchors.fill: parent
            keys: ["text/uri-list"]
            onEntered: (drag) => {
                if (root.connected) root.expanded = true;
            }
        }
    }

    // ── The island ──────────────────────────────────────────────────────────

    fullRepresentation: Item {
        // The page lays the island out in a fixed 720×320 window, glued to the
        // top edge and centred, exactly like the Windows and macOS panels.
        // In the popup: exactly the expanded island plus a margin, never less.
        // The desktop board fills whatever size the widget is given.
        readonly property real popupHeight: Math.min(320, root.islandHeight + 24)
        Layout.preferredWidth: root.inPanel ? 720 : Kirigami.Units.gridUnit * 22
        Layout.preferredHeight: root.inPanel ? popupHeight : Kirigami.Units.gridUnit * 30
        Layout.minimumWidth: root.inPanel ? 720 : Kirigami.Units.gridUnit * 14
        Layout.minimumHeight: root.inPanel ? popupHeight : Kirigami.Units.gridUnit * 12

        // On the desktop the page only does the thinking and the board below
        // draws (a Chromium canvas animating all day inside plasmashell grew
        // until the kernel OOM-killed it). It still has to be visible to run at
        // full speed, so it sits underneath at 1×1 and fully transparent.
        BoardView {
            anchors.fill: parent
            visible: !root.inPanel && root.connected
            board: root.board
            onOpenSession: (id) => root.toPage({ type: "board-open", payload: id })
            onOpenUrl: (url) => root.toPage({ type: "board-url", payload: url })
        }

        WebEngineView {
            id: view
            anchors.fill: root.inPanel ? parent : undefined
            width: root.inPanel ? parent.width : 1
            height: root.inPanel ? parent.height : 1
            opacity: root.inPanel ? 1 : 0
            z: -1
            visible: root.connected
            backgroundColor: "transparent"
            url: root.pageUrl
            settings.playbackRequiresUserGesture: false
            settings.showScrollBars: false
            settings.javascriptCanOpenWindows: false
            settings.localContentCanAccessFileUrls: false
            settings.localStorageEnabled: true
            Component.onCompleted: root.web = view

            onJavaScriptConsoleMessage: (level, message, lineNumber, sourceID) => {
                const prefix = "coucou-host:";
                if (!message.startsWith(prefix)) return;
                try {
                    root.fromPage(JSON.parse(message.slice(prefix.length)));
                } catch (e) {
                    console.warn("coucou: bad host message", e);
                }
            }

            // Links never navigate the island away: they go to the real browser.
            onNavigationRequested: (request) => {
                if (root.conn && request.url.toString().startsWith(root.conn.page)) return;
                request.reject();
                Qt.openUrlExternally(request.url);
            }
            onNewWindowRequested: (request) => Qt.openUrlExternally(request.requestedUrl)
            onContextMenuRequested: (request) => { request.accepted = true; }

            onLoadingChanged: (info) => {
                if (info.status === WebEngineView.LoadFailedStatus) root.reload();
            }

            // A click is how the chat field gets the keyboard on the desktop.
            TapHandler {
                gesturePolicy: TapHandler.WithinBounds
                grabPermissions: PointerHandler.ApprovesTakeOverByAnything
                onPressedChanged: if (pressed) view.forceActiveFocus()
            }
        }

        // Files dragged onto the island. Captured here, so QtWebEngine's own drop
        // handling never sees them, and handed to the page with real paths.
        DropArea {
            anchors.fill: parent
            keys: ["text/uri-list"]
            // The board takes no files.
            enabled: root.connected && root.inPanel
            onEntered: (drag) => {
                drag.accept(Qt.CopyAction);
                root.toPage({ type: "cursor", payload: { x: drag.x, y: drag.y } });
                root.toPage({ type: "drag", payload: { type: "enter" } });
            }
            onPositionChanged: (drag) => {
                root.toPage({ type: "cursor", payload: { x: drag.x, y: drag.y } });
                root.toPage({ type: "drag", payload: { type: "over" } });
            }
            onExited: root.toPage({ type: "drag", payload: { type: "leave" } })
            onDropped: (drop) => {
                const paths = [];
                for (const u of drop.urls) {
                    const s = u.toString();
                    if (s.startsWith("file://")) paths.push(decodeURIComponent(s.slice(7)));
                }
                drop.accept(Qt.CopyAction);
                root.toPage({ type: "drag", payload: { type: "drop", paths: paths } });
            }
        }

        // The app is not running.
        ColumnLayout {
            anchors.centerIn: parent
            visible: !root.connected
            spacing: Kirigami.Units.smallSpacing

            PanelMochi {
                Layout.alignment: Qt.AlignHCenter
                Layout.preferredWidth: Kirigami.Units.iconSizes.huge
                Layout.preferredHeight: Kirigami.Units.iconSizes.huge
                mood: "sleeping"
                awake: false
            }
            PlasmaComponents.Label {
                Layout.alignment: Qt.AlignHCenter
                text: i18n("Coucou isn't running")
            }
            PlasmaComponents.Button {
                Layout.alignment: Qt.AlignHCenter
                text: i18n("Start Coucou")
                icon.name: "media-playback-start"
                onClicked: root.startApp()
            }
        }
    }
}
