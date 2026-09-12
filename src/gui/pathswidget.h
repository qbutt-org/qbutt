/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QGroupBox>

class QComboBox;
class QLabel;
class QLineEdit;
class QListWidget;
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
    QComboBox *m_mode;
    QLineEdit *m_dnsServer;
    QLineEdit *m_bootstrapServer;
    QComboBox *m_dnsFamily;
    QPushButton *m_dnsApply;
    QListWidget *m_paths;
    QPushButton *m_refresh;
    QPushButton *m_localFile;
    QPushButton *m_start;
    QPushButton *m_disconnect;
    QPushButton *m_native;
    QLabel *m_status;
};
