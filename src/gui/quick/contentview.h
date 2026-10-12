// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QQuickWidget>

namespace Qbutt
{
    class ContentController;
}

namespace Content
{
    class ContentView final : public QQuickWidget
    {
        Q_OBJECT

    public:
        explicit ContentView(const QString &profileDir, QWidget *parent = nullptr);
        void giveFocusToSearchInput();
        bool isReading() const;

    signals:
        void torrentReady(const QString &magnet);
        void readerActiveChanged();
        void searchFinished(bool failed);

    private:
        void changeEvent(QEvent *event) override;

        Qbutt::ContentController *m_controller = nullptr;
    };
}
