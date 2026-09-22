// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QWidget>

class DownloadProgressOverlay final : public QWidget
{
    Q_OBJECT
    Q_PROPERTY(qreal progress READ progress)

public:
    DownloadProgressOverlay();

    qreal progress() const;

private:
    void refresh();
    void paintEvent(QPaintEvent *event) override;

    qreal m_progress = 0;
};
