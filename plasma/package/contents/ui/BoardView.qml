// The desktop board, drawn natively. The page (windows/src/board/main.ts) works
// out what it says and hands it over as `board`; this only draws it, with the
// panel's Mochi for every face. The look follows the island: near-black glass,
// hairlines, coloured status pills.

import QtQuick
import QtQuick.Layouts

Rectangle {
    id: view

    /** {hero, summary, usage[], rows[], chips[]} from the page, or null. */
    property var board: null
    property bool awake: true

    signal openSession(string id)
    signal openUrl(string url)

    readonly property color ink: "#F5F6F8"
    readonly property color dim: "#9398A1"
    readonly property color dim3: "#6B7079"
    readonly property color card: "#141518"
    readonly property color hover: "#1B1D21"
    readonly property color hairline: Qt.rgba(1, 1, 1, 0.06)

    function toneText(tone) {
        switch (tone) {
        case "amber": return "#F5A524";
        case "red": return "#FF8D97";
        case "green": return "#34D399";
        case "blue": return "#7CBCFF";
        case "violet": return "#C4B5FD";
        }
        return dim3;
    }
    function toneFill(tone) {
        switch (tone) {
        case "amber": return Qt.rgba(245 / 255, 165 / 255, 36 / 255, 0.14);
        case "red": return Qt.rgba(244 / 255, 80 / 255, 94 / 255, 0.16);
        case "green": return Qt.rgba(52 / 255, 211 / 255, 153 / 255, 0.12);
        case "blue": return Qt.rgba(59 / 255, 158 / 255, 255 / 255, 0.14);
        case "violet": return Qt.rgba(167 / 255, 139 / 255, 250 / 255, 0.14);
        }
        return "transparent";
    }
    function glowOf(mood) {
        switch (mood) {
        case "working": return "#3B9EFF";
        case "thinking": return "#A78BFA";
        case "searching": return "#6366F1";
        case "approval": return "#F5A524";
        case "error": return "#F4505E";
        case "finished": return "#34D399";
        case "ratelimit": return "#F59E0B";
        }
        return "#FFFFFF";
    }

    color: Qt.rgba(8 / 255, 9 / 255, 11 / 255, 0.9)
    radius: 22
    border.color: hairline
    border.width: 1

    // Rows keep their delegates (and their Mochi's blink) across updates.
    ListModel { id: rowModel }

    function syncRows(rows) {
        for (let i = rowModel.count - 1; i >= 0; i--) {
            if (!rows.some(r => r.key === rowModel.get(i).key)) rowModel.remove(i);
        }
        rows.forEach((r, i) => {
            let at = -1;
            for (let j = 0; j < rowModel.count; j++) {
                if (rowModel.get(j).key === r.key) { at = j; break; }
            }
            if (at < 0) {
                rowModel.insert(i, r);
                return;
            }
            if (at !== i) rowModel.move(at, i, 1);
            rowModel.set(i, r);
        });
    }

    onBoardChanged: syncRows(board ? board.rows : [])

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 12
        anchors.topMargin: 14
        spacing: 10

        // ── Head ────────────────────────────────────────────────────────────
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 4
            Layout.rightMargin: 4
            spacing: 10

            PanelMochi {
                Layout.preferredWidth: 52
                Layout.preferredHeight: 52
                mood: view.board ? view.board.hero : "sleeping"
                glow: view.glowOf(mood)
                awake: view.awake
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text {
                    text: "Claude Code"
                    color: view.ink
                    font.pixelSize: 15
                    font.weight: Font.DemiBold
                }
                Text {
                    Layout.fillWidth: true
                    text: view.board ? view.board.summary : ""
                    color: view.dim
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }
            }
        }

        // ── Usage ───────────────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            visible: !!view.board && view.board.usage.length > 0
            implicitHeight: usageRow.implicitHeight + 16
            color: view.card
            radius: 12
            border.color: view.hairline

            RowLayout {
                id: usageRow
                anchors.fill: parent
                anchors.margins: 8
                anchors.leftMargin: 10
                anchors.rightMargin: 10
                spacing: 12

                Repeater {
                    model: view.board ? view.board.usage : []
                    delegate: GridLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        Layout.preferredWidth: 1
                        columns: 3
                        columnSpacing: 7
                        rowSpacing: 3

                        Text {
                            text: modelData.label
                            color: view.dim
                            font.pixelSize: 11
                            font.weight: Font.DemiBold
                        }
                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 5
                            radius: 3
                            color: Qt.rgba(1, 1, 1, 0.08)
                            Rectangle {
                                width: parent.width * modelData.pct / 100
                                height: parent.height
                                radius: 3
                                color: modelData.tone === "red" ? "#F4505E"
                                    : modelData.tone === "amber" ? "#F5A524" : "#3B9EFF"
                            }
                        }
                        Text {
                            text: modelData.text
                            color: view.ink
                            font.pixelSize: 12
                            font.weight: Font.DemiBold
                        }
                        Item { implicitWidth: 1; implicitHeight: 1 }
                        Text {
                            Layout.columnSpan: 2
                            visible: modelData.reset !== ""
                            text: modelData.reset
                            color: view.dim3
                            font.pixelSize: 11
                        }
                    }
                }
            }
        }

        // ── Sessions ────────────────────────────────────────────────────────
        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 2
            model: rowModel
            boundsBehavior: Flickable.StopAtBounds
            move: Transition { NumberAnimation { properties: "y"; duration: 180; easing.type: Easing.OutQuad } }
            displaced: Transition { NumberAnimation { properties: "y"; duration: 180; easing.type: Easing.OutQuad } }

            delegate: MouseArea {
                id: row
                required property string key
                required property string name
                required property string detail
                required property string tip
                required property string status
                required property string tone
                required property string mood
                required property string tint
                required property bool wants
                required property bool quiet

                width: ListView.view.width
                height: Math.max(36, texts.implicitHeight + 12)
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: view.openSession(row.key)

                Rectangle {
                    anchors.fill: parent
                    radius: 10
                    color: row.wants ? Qt.rgba(245 / 255, 165 / 255, 36 / 255, 0.08)
                        : row.containsMouse ? view.hover : "transparent"
                    border.color: row.wants ? Qt.rgba(245 / 255, 165 / 255, 36 / 255, 0.22) : "transparent"
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 8
                    anchors.rightMargin: 8
                    spacing: 10

                    PanelMochi {
                        Layout.preferredWidth: 26
                        Layout.preferredHeight: 26
                        mood: row.mood
                        glow: view.glowOf(row.mood)
                        body: row.tint
                        awake: view.awake
                        hovered: row.containsMouse
                    }
                    ColumnLayout {
                        id: texts
                        Layout.fillWidth: true
                        spacing: 1
                        Text {
                            Layout.fillWidth: true
                            text: row.name
                            color: row.quiet ? "#C4C7CC" : view.ink
                            font.pixelSize: 13
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                        }
                        Text {
                            Layout.fillWidth: true
                            visible: row.detail !== ""
                            text: row.detail
                            color: row.quiet ? view.dim3 : view.dim
                            font.pixelSize: 12
                            elide: Text.ElideRight
                        }
                    }
                    Rectangle {
                        implicitWidth: statusText.implicitWidth + 16
                        implicitHeight: statusText.implicitHeight + 4
                        radius: height / 2
                        color: view.toneFill(row.tone)
                        Text {
                            id: statusText
                            anchors.centerIn: parent
                            text: row.status
                            color: view.toneText(row.tone)
                            font.pixelSize: 12
                            font.weight: Font.DemiBold
                        }
                    }
                }
            }

            Text {
                anchors.centerIn: parent
                width: parent.width - 32
                visible: rowModel.count === 0
                text: "Start Claude Code and its sessions show up here."
                color: view.dim3
                font.pixelSize: 13
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
            }
        }

        // ── Integrations ────────────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: view.hairline
            visible: chipRow.visible
        }
        RowLayout {
            id: chipRow
            Layout.fillWidth: true
            visible: !!view.board && view.board.chips.length > 0
            spacing: 8

            Repeater {
                model: view.board ? view.board.chips : []
                delegate: MouseArea {
                    id: chip
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    implicitHeight: chipText.implicitHeight + 16
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: view.openUrl(modelData.url)

                    Rectangle {
                        anchors.fill: parent
                        radius: 12
                        color: chip.containsMouse ? view.hover : view.card
                        border.color: view.hairline
                    }
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 10
                        anchors.rightMargin: 10
                        spacing: 7
                        Rectangle {
                            implicitWidth: 7
                            implicitHeight: 7
                            radius: 3.5
                            color: chip.modelData.color
                            opacity: chip.modelData.hot ? 1 : 0.45
                        }
                        Text {
                            text: chip.modelData.name
                            color: view.ink
                            font.pixelSize: 13
                            font.weight: Font.DemiBold
                        }
                        Text {
                            id: chipText
                            Layout.fillWidth: true
                            text: chip.modelData.text
                            color: chip.modelData.hot ? view.ink : view.dim
                            font.pixelSize: 13
                            elide: Text.ElideRight
                        }
                    }
                }
            }
        }
    }
}
