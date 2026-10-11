// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QQuickWidget>

namespace Content
{
    class ContentView final : public QQuickWidget
    {
        Q_OBJECT

    public:
        explicit ContentView(const QString &profileDir, QWidget *parent = nullptr);

    signals:
        void torrentReady(const QString &magnet);
    };
}
