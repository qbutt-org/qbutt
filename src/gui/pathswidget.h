/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QGroupBox>

class QComboBox;
class QLabel;
class QLineEdit;
class QPushButton;

namespace Net
{
    class PathManager;
}

class PathsWidget final : public QGroupBox
{
    Q_OBJECT

public:
    explicit PathsWidget(QWidget *parent = nullptr);

private:
    void refreshState();

    Net::PathManager *m_manager;
    QLineEdit *m_url;
    QComboBox *m_nodes;
    QComboBox *m_interfaces;
    QPushButton *m_refresh;
    QPushButton *m_localFile;
    QPushButton *m_start;
    QPushButton *m_native;
    QLabel *m_status;
};
