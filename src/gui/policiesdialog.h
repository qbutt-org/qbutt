/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QDialog>
#include <QJsonObject>

class QCheckBox;
class QLabel;
class QTableWidget;

class PoliciesDialog final : public QDialog
{
    Q_OBJECT

public:
    explicit PoliciesDialog(QWidget *parent = nullptr);

private:
    QJsonObject configuration() const;
    void addRule(const QJsonObject &rule);
    void refresh();

    QCheckBox *m_enabled;
    QCheckBox *m_deleteData;
    QTableWidget *m_rules;
    QTableWidget *m_preview;
    QTableWidget *m_journal;
    QLabel *m_status;
};
