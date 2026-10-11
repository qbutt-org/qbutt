// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QWidget>

class QComboBox;
class QHideEvent;
class QJsonObject;
class QJsonValue;
class QLabel;
class QLineEdit;
class QProgressBar;
class QPushButton;
class QShowEvent;
class QTimer;

namespace Content
{
    class BrowserConnectionWidget final : public QWidget
    {
        Q_OBJECT

    public:
        explicit BrowserConnectionWidget(bool chooseBrowser = false, QWidget *parent = nullptr);
        ~BrowserConnectionWidget() override;

    signals:
        void connected();

    protected:
        void showEvent(QShowEvent *event) override;
        void hideEvent(QHideEvent *event) override;

    private:
        void connectBrowser();
        void refreshStatus();
        void applyStatus(const QJsonObject &status);
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);

        const bool m_chooseBrowser;
        QWidget *m_browserRow = nullptr;
        QComboBox *m_browser = nullptr;
        QPushButton *m_connect = nullptr;
        QProgressBar *m_progress = nullptr;
        QLabel *m_status = nullptr;
        QLabel *m_setup = nullptr;
        QLineEdit *m_extensionDirectory = nullptr;
        QTimer *m_poll = nullptr;
        qint64 m_request = 0;
        bool m_connecting = false;
        bool m_installed = false;
        bool m_ready = false;
    };
}
