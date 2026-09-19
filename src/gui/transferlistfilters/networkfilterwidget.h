/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QWidget>

class QComboBox;
class TransferListWidget;

class NetworkFilterWidget final : public QWidget
{
    Q_OBJECT
    Q_DISABLE_COPY_MOVE(NetworkFilterWidget)

public:
    explicit NetworkFilterWidget(QWidget *parent, TransferListWidget *transferList);

public slots:
    void toggleFilter(bool enabled);

private:
    void refreshPaths();
    void applyPath();
    void applySource();

    TransferListWidget *m_transferList = nullptr;
    QComboBox *m_paths = nullptr;
    QComboBox *m_sources = nullptr;
    bool m_filterEnabled = false;
};
