// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QWidget>

class QLabel;

namespace Content
{
    class BrowserPrerequisiteWidget final : public QWidget
    {
        Q_OBJECT

    public:
        explicit BrowserPrerequisiteWidget(QWidget *parent = nullptr);
        void setSource(const QString &name);

    signals:
        void backRequested();
        void connected();

    private:
        QLabel *m_heading = nullptr;
    };
}
