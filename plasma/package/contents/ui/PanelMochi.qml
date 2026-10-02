// A small Mochi for the panel: the squircle, the two eyes, a blink, a breath,
// a bounce while Claude Code works and a badge when it needs you. The full
// character (emotes, particles, eye tracking) is drawn by the island itself.

import QtQuick

Item {
    id: mochi

    /** One of the island's BotStateName values. */
    property string mood: "idle"
    property color glow: "#FFFFFF"
    /** False when the app is not running or paused: eyes shut, no motion. */
    property bool awake: true
    property bool hovered: false
    /** The pill's colour, as on the island's mini Mochis. Unset: the grey Mochi. */
    property color body: "transparent"
    readonly property bool coloured: body.a > 0

    readonly property bool alerting: mood === "approval" || mood === "question" || mood === "error"
    readonly property bool busy: mood === "working" || mood === "thinking" || mood === "searching"
    readonly property bool asleep: !awake || mood === "sleeping"
    readonly property real s: Math.min(width, height)

    // Same palette as src/mochi/engine.ts.
    readonly property color baseTop: "#EDEDEF"
    readonly property color baseBottom: "#C4C5CA"
    readonly property color ink: coloured ? "#10131A" : "#1A1412"

    function tinted(c, amount) {
        return Qt.rgba(baseTop.r + (c.r - baseTop.r) * amount,
                       baseTop.g + (c.g - baseTop.g) * amount,
                       baseTop.b + (c.b - baseTop.b) * amount, 1);
    }

    // Soft halo in the state's colour.
    Rectangle {
        anchors.centerIn: bodyRect
        // Tight enough that Mochis sitting side by side don't run into each other.
        width: bodyRect.width * 1.12
        height: width
        radius: width / 2
        color: mochi.glow
        opacity: mochi.asleep || mochi.mood === "idle" ? 0 : (mochi.alerting ? mochi.pulse : 0.35)
        visible: opacity > 0
    }

    property real pulse: 0.5
    SequentialAnimation on pulse {
        running: mochi.alerting
        loops: Animation.Infinite
        NumberAnimation { to: 0.75; duration: 600; easing.type: Easing.InOutSine }
        NumberAnimation { to: 0.3; duration: 600; easing.type: Easing.InOutSine }
    }

    Rectangle {
        id: bodyRect
        width: mochi.s * 0.86
        height: width
        x: (mochi.width - width) / 2
        y: (mochi.height - height) / 2 + bounce.offset
        radius: width * 0.42
        antialiasing: true
        gradient: Gradient {
            GradientStop {
                position: 0
                color: mochi.coloured ? Qt.lighter(mochi.body, 1.25)
                    : mochi.mood === "idle" || mochi.asleep ? mochi.baseTop : mochi.tinted(mochi.glow, 0.55)
            }
            GradientStop { position: 1; color: mochi.coloured ? Qt.darker(mochi.body, 1.12) : mochi.baseBottom }
        }
        transform: Scale {
            origin.x: bodyRect.width / 2
            origin.y: bodyRect.height
            xScale: 1 + breath.value * 0.03
            yScale: 1 - breath.value * 0.03 + (mochi.hovered ? 0.04 : 0)
        }
        // Top highlight, as on the island.
        Rectangle {
            x: parent.width * 0.18
            y: parent.height * 0.08
            width: parent.width * 0.64
            height: parent.height * 0.3
            radius: height / 2
            color: "white"
            opacity: 0.35
        }

        Row {
            id: eyes
            anchors.horizontalCenter: parent.horizontalCenter
            y: parent.height * (0.5 - 0.12) - height / 2 + mochi.lookY
            spacing: bodyRect.width * 0.37 - eyeL.width
            Rectangle {
                id: eyeL
                width: bodyRect.width * (mochi.hovered ? 0.27 : 0.23)
                height: mochi.asleep ? Math.max(1.5, bodyRect.width * 0.05) : bodyRect.width * 0.27 * blink.open
                anchors.verticalCenter: parent.verticalCenter
                radius: Math.min(width, height) / 2
                color: mochi.ink
                antialiasing: true
            }
            Rectangle {
                width: eyeL.width
                height: eyeL.height
                anchors.verticalCenter: parent.verticalCenter
                radius: eyeL.radius
                color: mochi.ink
                antialiasing: true
            }
        }
    }

    // Badge: "!" for errors, "?" for questions, a dot for approvals.
    Rectangle {
        visible: mochi.alerting && mochi.awake
        width: mochi.s * 0.36
        height: width
        radius: width / 2
        x: bodyRect.x + bodyRect.width - width * 0.7
        y: bodyRect.y - height * 0.25
        color: mochi.glow
        border.color: "#000000"
        border.width: Math.max(1, mochi.s * 0.03)
        Text {
            anchors.centerIn: parent
            text: mochi.mood === "error" ? "!" : mochi.mood === "question" ? "?" : ""
            color: "#000000"
            font.bold: true
            font.pixelSize: parent.height * 0.8
        }
    }

    // Eyes drift while Claude searches, look down while it works.
    readonly property real lookY: mood === "working" ? bodyRect.height * 0.04
        : mood === "thinking" ? -bodyRect.height * 0.04 : 0

    QtObject {
        id: blink
        property real open: 1
    }
    SequentialAnimation {
        running: !mochi.asleep
        loops: Animation.Infinite
        PauseAnimation { duration: 2600 + Math.random() * 3000 }
        NumberAnimation { target: blink; property: "open"; to: 0.1; duration: 70 }
        NumberAnimation { target: blink; property: "open"; to: 1; duration: 110 }
    }

    QtObject {
        id: breath
        property real value: 0
    }
    SequentialAnimation {
        running: mochi.awake && !mochi.busy
        loops: Animation.Infinite
        NumberAnimation { target: breath; property: "value"; to: 1; duration: 1800; easing.type: Easing.InOutSine }
        NumberAnimation { target: breath; property: "value"; to: 0; duration: 1800; easing.type: Easing.InOutSine }
    }

    QtObject {
        id: bounce
        property real offset: 0
    }
    SequentialAnimation {
        running: mochi.awake && mochi.busy
        loops: Animation.Infinite
        onRunningChanged: if (!running) bounce.offset = 0
        NumberAnimation { target: bounce; property: "offset"; to: -mochi.s * 0.08; duration: 260; easing.type: Easing.OutQuad }
        NumberAnimation { target: bounce; property: "offset"; to: 0; duration: 260; easing.type: Easing.InQuad }
        PauseAnimation { duration: 120 }
    }
}
