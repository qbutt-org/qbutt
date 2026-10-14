// SPDX-License-Identifier: GPL-2.0-or-later
pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import Qbutt.Content

ModalPicker {
    id: root
    objectName: "readerTranslationPicker"

    required property ContentController backend
    property string service: "codex"
    property bool removeKey: false
    readonly property bool openRouter: backend.readerIsText && service === "openrouter"
    readonly property var openRouterEfforts: [
        { title: qsTr("Default"), id: "default" },
        { title: qsTr("None"), id: "none" },
        { title: qsTr("Minimal"), id: "minimal" },
        { title: qsTr("Low"), id: "low" },
        { title: qsTr("Medium"), id: "medium" },
        { title: qsTr("High"), id: "high" },
        { title: qsTr("Very high"), id: "xhigh" },
        { title: qsTr("Maximum"), id: "max" }
    ]

    function syncSettings(): void {
        service = backend.readerTextTranslationService;
        openRouterModel.text = backend.readerOpenRouterModel;
        openRouterEffort.currentIndex = openRouterEfforts.findIndex(choice =>
            choice.id === backend.readerOpenRouterEffort);
        apiKey.clear();
        removeKey = false;
    }

    Connections {
        target: root.backend
        function onReaderTranslationOptionsChanged(): void {
            if (!root.backend.readerTranslationOptionsLoading
                    && root.backend.readerTranslationSettingsError === Reader.NoTranslationSettingsError)
                root.syncSettings();
        }
    }

    readonly property var modelChoices: {
        const choices = [{ title: qsTr("Automatic"), id: "" }];
        for (let index = 0; index < backend.readerTranslationModelCount; ++index) {
            const model = backend.readerTranslationModelAt(index);
            choices.push({ title: model, id: model });
        }
        return choices;
    }
    readonly property var effortChoices: {
        const choices = [];
        const labels = {
            low: qsTr("Low"),
            medium: qsTr("Medium"),
            high: qsTr("High"),
            xhigh: qsTr("Very high"),
            max: qsTr("Maximum"),
            ultra: qsTr("Ultra")
        };
        for (let index = 0; index < backend.readerTranslationEffortCount; ++index) {
            const effort = backend.readerTranslationEffortAt(index);
            choices.push({ title: labels[effort] ?? effort, id: effort });
        }
        return choices;
    }

    anchors.centerIn: undefined
    width: Math.min(420, parent.width - 48)
    height: Math.min(contentItem.implicitHeight + topPadding + bottomPadding, parent.height - 48)
    x: parent.width - width - 24
    y: (parent.height - height) / 2

    onOpened: {
        syncSettings();
        backend.loadReaderTranslationOptions();
    }
    onClosed: apiKey.clear()

    contentItem: ColumnLayout {
        spacing: 16

        RowLayout {
            Layout.fillWidth: true

            Label {
                Layout.fillWidth: true
                text: qsTr("Translation settings")
                font.pixelSize: 22
                font.weight: Font.DemiBold
            }

            ReaderChromeButton {
                objectName: "readerTranslationClose"
                theme: root.theme
                foreground: root.theme.foreground
                icon.source: root.theme.iconSource("close")
                horizontalPadding: 0
                Layout.preferredWidth: 42
                Accessible.name: qsTr("Close")
                onClicked: root.close()
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: 2
            columnSpacing: 16
            rowSpacing: 16

            Label { text: qsTr("Translate into") }
            SelectField {
                objectName: "readerTranslationLanguage"
                Layout.fillWidth: true
                theme: root.theme
                enabled: root.backend.readerSettingsState === Reader.Ready
                Accessible.name: qsTr("Translation language")
                textRole: "title"
                valueRole: "id"
                model: [
                    { title: qsTr("Russian"), id: "ru" },
                    { title: qsTr("English"), id: "en" }
                ]
                currentIndex: root.backend.readerTranslationLanguage === "en" ? 1 : 0
                onActivated: {
                    if (root.backend.setReaderTranslationLanguage(currentValue))
                        root.backend.loadReaderTranslationOptions();
                }
            }

            Label {
                visible: root.backend.readerIsText
                text: qsTr("Service")
            }
            SelectField {
                objectName: "readerTranslationService"
                Layout.fillWidth: true
                visible: root.backend.readerIsText
                theme: root.theme
                enabled: !root.backend.readerTranslationOptionsLoading
                Accessible.name: qsTr("Translation service")
                textRole: "title"
                valueRole: "id"
                model: [
                    { title: "Codex", id: "codex" },
                    { title: "OpenRouter", id: "openrouter" }
                ]
                currentIndex: root.service === "openrouter" ? 1 : 0
                onActivated: root.service = currentValue
            }

            Label {
                visible: !root.openRouter
                text: qsTr("Codex model")
            }
            SelectField {
                objectName: "readerTranslationModel"
                Layout.fillWidth: true
                visible: !root.openRouter
                theme: root.theme
                enabled: !root.backend.readerTranslationOptionsLoading
                    && (root.backend.readerTranslationModelCount > 0
                        || root.backend.readerTranslationModel !== "")
                Accessible.name: qsTr("Codex model")
                textRole: "title"
                valueRole: "id"
                model: root.modelChoices
                currentIndex: root.modelChoices.findIndex(choice =>
                    choice.id === root.backend.readerTranslationModel)
                displayText: currentIndex < 0 ? root.backend.readerTranslationModel : currentText
                onActivated: root.backend.setReaderTranslationModel(currentValue)
            }

            Label {
                visible: codexEffort.visible
                text: qsTr("Reasoning")
            }
            SelectField {
                id: codexEffort
                objectName: "readerTranslationEffort"
                Layout.fillWidth: true
                visible: !root.openRouter && root.backend.readerTranslationModel !== ""
                theme: root.theme
                enabled: !root.backend.readerTranslationOptionsLoading
                    && !root.backend.readerTranslationOptionsError
                    && root.backend.readerTranslationEffortCount > 0
                Accessible.name: qsTr("Reasoning effort")
                textRole: "title"
                valueRole: "id"
                model: root.effortChoices
                currentIndex: root.effortChoices.findIndex(choice =>
                    choice.id === root.backend.readerTranslationEffort)
                displayText: currentIndex < 0 ? root.backend.readerTranslationEffort : currentText
                onActivated: root.backend.setReaderTranslationEffort(currentValue)
            }

            Label {
                visible: root.openRouter
                text: qsTr("Model")
            }
            QueryField {
                id: openRouterModel
                objectName: "readerOpenRouterModel"
                Layout.fillWidth: true
                visible: root.openRouter
                enabled: !root.backend.readerTranslationOptionsLoading
                theme: root.theme
                maximumLength: 128
                placeholderText: "author/model"
                Accessible.name: qsTr("OpenRouter model")
            }

            Label {
                objectName: "readerTranslationSettingsError"
                Layout.columnSpan: 2
                Layout.fillWidth: true
                visible: root.openRouter
                    && root.backend.readerTranslationSettingsError === Reader.InvalidTranslationModel
                text: qsTr("Enter a model in author/model format.")
                color: root.theme.muted
                wrapMode: Text.Wrap
            }

            Label {
                visible: root.openRouter
                text: qsTr("Reasoning")
            }
            SelectField {
                id: openRouterEffort
                objectName: "readerOpenRouterEffort"
                Layout.fillWidth: true
                visible: root.openRouter
                enabled: !root.backend.readerTranslationOptionsLoading
                theme: root.theme
                textRole: "title"
                valueRole: "id"
                model: root.openRouterEfforts
                Accessible.name: qsTr("Reasoning effort")
            }

            Label {
                visible: root.openRouter
                text: qsTr("API key")
            }
            RowLayout {
                Layout.fillWidth: true
                visible: root.openRouter
                enabled: !root.backend.readerTranslationOptionsLoading
                spacing: 8

                QueryField {
                    id: apiKey
                    objectName: "readerOpenRouterKey"
                    Layout.fillWidth: true
                    theme: root.theme
                    echoMode: TextInput.Password
                    maximumLength: 8192
                    placeholderText: root.backend.readerOpenRouterKeyConfigured && !root.removeKey
                        ? qsTr("Saved key") : qsTr("API key")
                    Accessible.name: qsTr("OpenRouter API key")
                    onTextEdited: root.removeKey = false
                }

                ActionButton {
                    objectName: "readerOpenRouterRemoveKey"
                    theme: root.theme
                    text: qsTr("Remove")
                    visible: root.backend.readerOpenRouterKeyConfigured && !root.removeKey
                    onClicked: {
                        apiKey.clear();
                        root.removeKey = true;
                    }
                }
            }
        }
        Label {
            objectName: "readerTranslationSaveError"
            Layout.fillWidth: true
            visible: root.backend.readerTranslationSettingsError !== Reader.NoTranslationSettingsError
                && root.backend.readerTranslationSettingsError !== Reader.InvalidTranslationModel
            text: {
                switch (root.backend.readerTranslationSettingsError) {
                case Reader.TranslationKeyProtectionFailed: return qsTr("Windows could not protect the API key.");
                default: return qsTr("Translation settings could not be saved.");
                }
            }
            color: root.theme.muted
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            visible: !root.openRouter && root.backend.readerTranslationOptionsError

            Label {
                Layout.fillWidth: true
                text: qsTr("Translation options could not be loaded.")
                color: root.theme.muted
                wrapMode: Text.Wrap
            }

            ActionButton {
                objectName: "readerTranslationOptionsRetry"
                theme: root.theme
                text: qsTr("Try again")
                onClicked: root.backend.loadReaderTranslationOptions()
            }
        }

        ActionButton {
            objectName: "readerTranslationApply"
            Layout.alignment: Qt.AlignRight
            visible: root.backend.readerIsText
            enabled: !root.backend.readerTranslationOptionsLoading
            theme: root.theme
            highlighted: true
            text: qsTr("Apply")
            onClicked: {
                root.backend.configureReaderTextTranslation(root.service,
                    openRouterModel.text.trim(), openRouterEffort.currentValue,
                    apiKey.text, root.removeKey);
            }
        }
    }
}
