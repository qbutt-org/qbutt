// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "contentBrowserSettings"

    required property ContentController backend
    signal connected()

    property bool announcedConnection: false

    width: Math.min(500, parent.width - 48)
    height: Math.min(contentItem.implicitHeight + topPadding + bottomPadding, parent.height - 48)

    onOpened: {
        announcedConnection = false;
        backend.watchBrowser();
    }
    onClosed: backend.closeBrowserSettings()

    Connections {
        target: root.backend
        function onBrowserChanged() {
            if (root.visible && !root.announcedConnection
                && root.backend.browserConnectionState === Browser.Connected) {
                root.announcedConnection = true;
                root.connected();
            }
        }
    }

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            spacing: 16

            Label {
                Layout.fillWidth: true
                text: qsTr("Browser")
                font.pixelSize: 24
                font.weight: Font.DemiBold
            }

            ActionButton {
                objectName: "contentBrowserClose"
                theme: root.theme
                text: qsTr("Close")
                onClicked: root.close()
            }
        }

        SelectField {
            id: browserChoice
            objectName: "contentBrowserChoice"
            Layout.fillWidth: true
            theme: root.theme
            model: [
                { text: qsTr("Automatic"), value: "auto" },
                { text: "Microsoft Edge", value: "edge" },
                { text: "Google Chrome", value: "chrome" }
            ]
            textRole: "text"
            valueRole: "value"
            currentIndex: root.backend.browserChoice === "edge" ? 1
                : root.backend.browserChoice === "chrome" ? 2 : 0
            Accessible.name: qsTr("Browser")
            onActivated: root.backend.chooseBrowser(currentValue)
        }

        Label {
            id: status
            objectName: "contentBrowserStatus"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            color: root.theme.foreground
            text: {
                if (root.backend.browserSetupState === Browser.Busy)
                    return qsTr("Preparing qcatch…");
                if (root.backend.browserSetupState === Browser.Failed) {
                    switch (root.backend.browserSetupFailure) {
                    case Browser.Integrity: return qsTr("The qcatch download failed its integrity check.");
                    case Browser.Download: return qsTr("Could not download qcatch. Check your connection and try again.");
                    case Browser.Install: return qsTr("Could not install qcatch. Try again.");
                    default: return qsTr("Could not open qcatch setup. Try again.");
                    }
                }
                if (root.backend.browserConnectionState === Browser.Connected)
                    return qsTr("qcatch connected in %1.").arg(root.backend.browserConnectedName);
                if (root.backend.browserConnectionState === Browser.OwnerExists)
                    return qsTr("Another qbutt profile owns the browser connection. Close it and retry.");
                if (root.backend.browserConnectionState === Browser.Unavailable)
                    return qsTr("The browser connection is unavailable.");
                if (root.backend.browserConnectionState === Browser.NoBrowser) {
                    if (root.backend.browserChoice === "edge")
                        return qsTr("Microsoft Edge is not installed.");
                    if (root.backend.browserChoice === "chrome")
                        return qsTr("Google Chrome is not installed.");
                    return qsTr("Install Microsoft Edge or Google Chrome to use qcatch.");
                }
                if (root.backend.browserConnectionState === Browser.ChoiceRequired)
                    return qsTr("Choose Microsoft Edge or Google Chrome.");
                if (root.backend.browserSetupState === Browser.Waiting)
                    return qsTr("Waiting for the qcatch extension…");
                if (root.backend.browserConnectionState === Browser.Checking)
                    return qsTr("Checking connection…");
                return qsTr("qcatch is not connected.");
            }
        }

        Label {
            Layout.fillWidth: true
            visible: root.backend.browserSetupState === Browser.Waiting
                && root.backend.browserConnectionState !== Browser.Connected
            text: qsTr("In Extensions, turn on Developer mode and choose Load unpacked:")
            wrapMode: Text.Wrap
        }

        QueryField {
            objectName: "contentBrowserExtensionDirectory"
            Layout.fillWidth: true
            theme: root.theme
            visible: root.backend.browserSetupState === Browser.Waiting
                && root.backend.browserConnectionState !== Browser.Connected
            text: root.backend.browserExtensionDirectory
            readOnly: true
            selectByMouse: true
            Accessible.name: qsTr("qcatch extension folder")
        }

        ActionButton {
            objectName: "contentBrowserAction"
            theme: root.theme
            visible: root.backend.browserSetupState !== Browser.Busy
                && (root.backend.browserConnectionState === Browser.Disconnected
                    || root.backend.browserConnectionState === Browser.OwnerExists
                    || root.backend.browserConnectionState === Browser.Unavailable)
            enabled: visible
            text: root.backend.browserConnectionState === Browser.OwnerExists
                || root.backend.browserConnectionState === Browser.Unavailable
                ? qsTr("Retry") : root.backend.browserSetupState === Browser.Waiting
                    ? qsTr("Open setup") : root.backend.browserSetupState === Browser.Failed
                        ? qsTr("Try again") : qsTr("Connect qcatch")
            onClicked: {
                if (root.backend.browserConnectionState === Browser.OwnerExists
                    || root.backend.browserConnectionState === Browser.Unavailable)
                    root.backend.watchBrowser();
                else
                    root.backend.connectQcatch();
            }
        }
    }
}
