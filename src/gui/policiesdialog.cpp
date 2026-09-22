/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "policiesdialog.h"

#include <algorithm>
#include <utility>

#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QHeaderView>
#include <QLabel>
#include <QMessageBox>
#include <QPushButton>
#include <QTableWidget>
#include <QTabWidget>
#include <QTextDocument>
#include <QUuid>
#include <QVBoxLayout>

#include "base/bittorrent/completionpolicy.h"
#include "base/bittorrent/session.h"
#include "base/global.h"

namespace
{
    QString joinValues(const QJsonArray &values)
    {
        QStringList names;
        for (const QJsonValue &value : values)
            names.append(value.toString());
        return names.join(u", "_s);
    }

    QString actionNames(const QJsonArray &actions)
    {
        QStringList names;
        for (const QJsonValue &value : actions)
        {
            const QString action = value.toString();
            if (action == u"stop")
                names.append(PoliciesDialog::tr("Stop"));
            else if (action == u"remove_torrent")
                names.append(PoliciesDialog::tr("Remove torrent (keep files)"));
            else if (action == u"delete_data")
                names.append(PoliciesDialog::tr("Delete data and remove torrent"));
            else if (action == u"notify")
                names.append(PoliciesDialog::tr("Notify"));
        }
        return names.join(u", "_s);
    }
}

PoliciesDialog::PoliciesDialog(QWidget *parent)
    : QDialog(parent)
    , m_enabled(new QCheckBox(tr("Enable completion rules"), this))
    , m_deleteData(new QCheckBox(tr("Allow rules to delete downloaded files"), this))
    , m_rules(new QTableWidget(0, 8, this))
    , m_preview(new QTableWidget(0, 5, this))
    , m_journal(new QTableWidget(0, 6, this))
    , m_status(new QLabel(this))
    , m_empty(new QLabel(tr("No completion rules"), this))
    , m_tabs(new QTabWidget(this))
    , m_remove(new QPushButton(tr("Remove"), this))
    , m_up(new QPushButton(tr("Move up"), this))
    , m_down(new QPushButton(tr("Move down"), this))
    , m_accept(new QPushButton(tr("Allow rules for this torrent…"), this))
{
    setObjectName(u"completionPoliciesDialog"_s);
    setWindowTitle(tr("Completion rules"));
    m_enabled->setObjectName(u"completionPoliciesEnabled"_s);
    m_deleteData->setObjectName(u"completionPoliciesAllowDelete"_s);
    m_rules->setObjectName(u"completionPoliciesRules"_s);
    m_preview->setObjectName(u"completionPoliciesPreview"_s);
    m_journal->setObjectName(u"completionPoliciesJournal"_s);
    m_status->setObjectName(u"completionPoliciesStatus"_s);
    m_empty->setObjectName(u"completionPoliciesEmpty"_s);
    m_tabs->setObjectName(u"completionPoliciesTabs"_s);
    m_accept->setObjectName(u"completionPoliciesAccept"_s);
    auto *layout = new QVBoxLayout(this);
    layout->addWidget(m_enabled);
    layout->addWidget(m_deleteData);
    m_enabled->setToolTip(tr("Run rules in order after downloads are verified. The first Stop, Remove or Delete action ends evaluation."));
    layout->addWidget(m_empty);
    auto *rulePage = new QWidget(m_tabs);
    auto *ruleLayout = new QVBoxLayout(rulePage);
    m_rules->setHorizontalHeaderLabels({tr("Rule"), tr("Enabled"), tr("Category"), tr("Tags"), tr("Min ratio")
        , tr("Seeding (seconds)"), tr("Action"), tr("Notify")});
    m_rules->hideColumn(0);
    m_rules->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_rules->setSelectionMode(QAbstractItemView::SingleSelection);
    m_rules->horizontalHeaderItem(2)->setToolTip(tr("Use * for all categories."));
    m_rules->horizontalHeaderItem(3)->setToolTip(tr("Separate tags with commas."));
    ruleLayout->addWidget(m_rules);
    auto *ruleButtons = new QDialogButtonBox(this);
    auto *add = ruleButtons->addButton(tr("Add rule"), QDialogButtonBox::ActionRole);
    add->setObjectName(u"completionPoliciesAdd"_s);
    ruleButtons->addButton(m_remove, QDialogButtonBox::ActionRole);
    ruleButtons->addButton(m_up, QDialogButtonBox::ActionRole);
    ruleButtons->addButton(m_down, QDialogButtonBox::ActionRole);
    m_tabs->addTab(rulePage, tr("Rules"));
    auto *previewPage = new QWidget(m_tabs);
    auto *previewLayout = new QVBoxLayout(previewPage);
    m_preview->setHorizontalHeaderLabels({tr("Torrent"), tr("Rule"), tr("Actions"), tr("Status"), tr("Permission")});
    m_preview->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_preview->setSelectionMode(QAbstractItemView::SingleSelection);
    m_preview->setEditTriggers(QAbstractItemView::NoEditTriggers);
    previewLayout->addWidget(m_preview);
    previewLayout->addWidget(m_accept);
    m_tabs->addTab(previewPage, tr("Preview"));
    m_journal->setHorizontalHeaderLabels({tr("Time"), tr("Torrent"), tr("Rule"), tr("Actions"), tr("Result"), tr("Reason")});
    m_journal->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_tabs->addTab(m_journal, tr("History"));
    layout->addWidget(m_tabs);
    layout->addWidget(ruleButtons);
    m_status->setTextFormat(Qt::PlainText);
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Save | QDialogButtonBox::Close, this);
    auto *preview = buttons->addButton(tr("Refresh"), QDialogButtonBox::ActionRole);
    preview->setObjectName(u"completionPoliciesRefresh"_s);
    buttons->button(QDialogButtonBox::Save)->setObjectName(u"completionPoliciesSave"_s);
    layout->addWidget(buttons);
    auto *policy = BitTorrent::Session::instance()->completionPolicy();
    const QJsonObject config = policy->configuration();
    m_enabled->setChecked(config[u"enabled"_s].toBool());
    m_deleteData->setChecked(config[u"allow_delete_data"_s].toBool());
    for (const QJsonValue &rule : config[u"rules"_s].toArray())
        addRule(rule.toObject());
    connect(add, &QPushButton::clicked, this, [this]
    {
        addRule({});
        m_tabs->setCurrentIndex(0);
        m_rules->setCurrentCell(m_rules->rowCount() - 1, 2);
        updateControls();
        resize(1000, 560);
    });
    connect(m_remove, &QPushButton::clicked, this, [this]
    {
        m_rules->removeRow(m_rules->currentRow());
        updateControls();
    });
    connect(m_rules, &QTableWidget::itemSelectionChanged, this, &PoliciesDialog::updateControls);
    connect(m_preview, &QTableWidget::itemSelectionChanged, this, &PoliciesDialog::updateControls);
    const auto move = [this](const int offset)
    {
        const int row = m_rules->currentRow();
        QJsonArray rules = configuration()[u"rules"_s].toArray();
        if ((row < 0) || (row + offset < 0) || (row + offset >= rules.size()))
            return;
        const QJsonValue value = rules.takeAt(row);
        rules.insert(row + offset, value);
        m_rules->setRowCount(0);
        for (const QJsonValue &rule : rules)
            addRule(rule.toObject());
        m_rules->setCurrentCell(row + offset, 2);
    };
    connect(m_up, &QPushButton::clicked, this, [move] { move(-1); });
    connect(m_down, &QPushButton::clicked, this, [move] { move(1); });
    connect(m_tabs, &QTabWidget::currentChanged, this, [this, preview](const int index)
    {
        preview->setVisible(index == 1);
        updateControls();
    });
    preview->hide();
    connect(preview, &QPushButton::clicked, this, &PoliciesDialog::refresh);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(buttons, &QDialogButtonBox::accepted, this, [this, policy]
    {
        refresh();
        if (const QString error = policy->validateConfiguration(configuration()); !error.isEmpty())
        {
            m_status->setText(error);
            return;
        }
        if (m_enabled->isChecked() && (QMessageBox::question(this, tr("Enable completion rules")
            , tr("Save and enable these rules? Imported torrents need separate permission.")) != QMessageBox::Yes))
            return;
        const QString error = policy->configure(configuration());
        m_status->setText(error.isEmpty() ? tr("Rules saved.") : error);
    });
    connect(m_accept, &QPushButton::clicked, this, [this, policy]
    {
        const int row = m_preview->currentRow();
        if (row < 0)
            return;
        if (QMessageBox::question(this, tr("Allow completion rules")
            , tr("Allow saved rules for this stopped torrent? Rules may remove it or delete its files if deletion is allowed.")) != QMessageBox::Yes)
            return;
        const auto id = BitTorrent::TorrentID::fromString(m_preview->item(row, 0)->data(Qt::UserRole).toString());
        const QString error = policy->acknowledgePreview(id);
        m_status->setText(error.isEmpty() ? tr("Saving permission…") : error);
    });
    connect(policy, &BitTorrent::CompletionPolicy::changed, this, &PoliciesDialog::refresh);
    refresh();
    if (m_rules->rowCount() > 0 || m_journal->rowCount() > 0)
        resize(1000, 560);
    else
        resize(680, sizeHint().height());
}

void PoliciesDialog::addRule(const QJsonObject &rule)
{
    const int row = m_rules->rowCount();
    m_rules->insertRow(row);
    const QJsonObject match = rule[u"match"_s].toObject();
    const QJsonArray actions = rule[u"actions"_s].toArray();
    m_rules->setItem(row, 0, new QTableWidgetItem(rule[u"id"_s].toString(QUuid::createUuid().toString(QUuid::WithoutBraces))));
    auto *enabled = new QCheckBox(this);
    enabled->setChecked(rule[u"enabled"_s].toBool(true));
    m_rules->setCellWidget(row, 1, enabled);
    m_rules->setItem(row, 2, new QTableWidgetItem(match[u"category"_s].toString(u"*"_s)));
    m_rules->setItem(row, 3, new QTableWidgetItem(joinValues(match[u"tags"_s].toArray())));
    m_rules->setItem(row, 4, new QTableWidgetItem(QString::number(match[u"min_ratio"_s].toDouble())));
    m_rules->setItem(row, 5, new QTableWidgetItem(QString::number(match[u"min_seeding_seconds"_s].toDouble())));
    auto *action = new QComboBox(this);
    action->addItem(tr("None"), QString());
    action->addItem(tr("Stop"), u"stop"_s);
    action->addItem(tr("Remove torrent (keep files)"), u"remove_torrent"_s);
    action->addItem(tr("Delete data and remove torrent"), u"delete_data"_s);
    for (int index = 1; index < action->count(); ++index)
    {
        if (actions.contains(action->itemData(index).toString()))
            action->setCurrentIndex(index);
    }
    m_rules->setCellWidget(row, 6, action);
    auto *notify = new QCheckBox(this);
    notify->setChecked(rule.isEmpty() || actions.contains(u"notify"_s));
    m_rules->setCellWidget(row, 7, notify);
    connect(action, &QComboBox::currentIndexChanged, this, &PoliciesDialog::updateControls);
}

QJsonObject PoliciesDialog::configuration() const
{
    QJsonArray rules;
    for (int row = 0; row < m_rules->rowCount(); ++row)
    {
        QJsonObject match;
        if (m_rules->item(row, 2)->text() != u"*")
            match[u"category"_s] = m_rules->item(row, 2)->text();
        QStringList tags = m_rules->item(row, 3)->text().split(u',', Qt::SkipEmptyParts);
        for (QString &tag : tags)
            tag = tag.trimmed();
        match[u"tags"_s] = QJsonArray::fromStringList(tags);
        for (const auto &[column, key] : {std::pair {4, u"min_ratio"_s}, std::pair {5, u"min_seeding_seconds"_s}})
        {
            bool valid = false;
            const double number = m_rules->item(row, column)->text().toDouble(&valid);
            match[key] = valid ? QJsonValue(number) : QJsonValue(m_rules->item(row, column)->text());
        }
        QJsonArray actions;
        const QString terminal = static_cast<QComboBox *>(m_rules->cellWidget(row, 6))->currentData().toString();
        if (!terminal.isEmpty())
            actions.append(terminal);
        if (static_cast<QCheckBox *>(m_rules->cellWidget(row, 7))->isChecked())
            actions.append(u"notify"_s);
        rules.append(QJsonObject {{u"id"_s, m_rules->item(row, 0)->text()}
            , {u"enabled"_s, static_cast<QCheckBox *>(m_rules->cellWidget(row, 1))->isChecked()}
            , {u"match"_s, match}, {u"actions"_s, actions}});
    }
    return {{u"enabled"_s, m_enabled->isChecked()}, {u"allow_delete_data"_s, m_deleteData->isChecked()}, {u"rules"_s, rules}};
}

void PoliciesDialog::refresh()
{
    const auto *policy = BitTorrent::Session::instance()->completionPolicy();
    m_preview->setRowCount(0);
    for (const QJsonValue &value : policy->preview(configuration()))
    {
        const QJsonObject torrent = value.toObject();
        QJsonArray rules = torrent[u"rules"_s].toArray();
        if (rules.isEmpty())
            rules.append(QJsonObject());
        for (const QJsonValue &ruleValue : rules)
        {
            const QJsonObject rule = ruleValue.toObject();
            const int row = m_preview->rowCount();
            m_preview->insertRow(row);
            m_preview->setItem(row, 0, new QTableWidgetItem(torrent[u"name"_s].toString()));
            m_preview->item(row, 0)->setData(Qt::UserRole, torrent[u"hash"_s].toString());
            const QString ruleId = rule[u"rule"_s].toString();
            QString ruleName = tr("No match");
            for (int index = 0; index < m_rules->rowCount(); ++index)
            {
                if (m_rules->item(index, 0)->text() == ruleId)
                    ruleName = tr("Rule %L1").arg(index + 1);
            }
            auto *ruleItem = new QTableWidgetItem(ruleName);
            ruleItem->setToolTip(ruleId);
            m_preview->setItem(row, 1, ruleItem);
            m_preview->setItem(row, 2, new QTableWidgetItem(actionNames(rule[u"actions"_s].toArray())));
            m_preview->setItem(row, 3, new QTableWidgetItem(rule[u"already_claimed"_s].toBool() ? tr("Recorded; see History")
                : torrent[u"recheck_paused"_s].toBool() ? tr("Start torrent to verify files")
                : torrent[u"ready"_s].toBool() ? tr("Ready") : tr("Waiting for files")));
            auto *permission = new QTableWidgetItem(torrent[u"preview_required"_s].toBool() ? tr("Required") : tr("Allowed"));
            permission->setData(Qt::UserRole, torrent[u"preview_required"_s].toBool());
            m_preview->setItem(row, 4, permission);
        }
    }
    const QJsonArray journal = policy->journal();
    m_journal->setRowCount(0);
    for (qsizetype index = std::max<qsizetype>(0, journal.size() - 500); index < journal.size(); ++index)
    {
        const QJsonObject entry = journal[index].toObject();
        const int row = m_journal->rowCount();
        m_journal->insertRow(row);
        const QJsonObject reason = entry[u"reason"_s].toObject();
        const QString details = tr("Category: %1; tags: %2; ratio: %3; seeding: %4 s")
            .arg(reason[u"category"_s].toString(), joinValues(reason[u"tags"_s].toArray())
                , QString::number(reason[u"ratio"_s].toDouble()), QString::number(reason[u"seeding_seconds"_s].toInteger()));
        const QString status = entry[u"status"_s].toString();
        const QString result = (status == u"claimed") ? tr("Interrupted; review required")
            : (status == u"blocked") ? tr("Blocked; review required") : tr("Action submitted");
        const QStringList values {entry[u"time"_s].toString(), entry[u"name"_s].toString(), entry[u"rule"_s].toString()
            , actionNames(entry[u"actions"_s].toArray()), result, details};
        for (int column = 0; column < values.size(); ++column)
            m_journal->setItem(row, column, new QTableWidgetItem(values[column]));
        m_journal->item(row, 5)->setToolTip(Qt::convertFromPlainText(details + u'\n' + reason[u"destination"_s].toString()));
        m_journal->item(row, 4)->setToolTip(tr("Claimed actions are never repeated automatically after a crash. "
            "Dispatched records mean the action was submitted to the native session. Key: %1").arg(entry[u"key"_s].toString()));
    }
    for (QTableWidget *table : {m_rules, m_preview, m_journal})
    {
        table->resizeColumnsToContents();
        table->horizontalHeader()->setStretchLastSection(true);
    }
    if (const QString error = policy->configuration()[u"error"_s].toString(); !error.isEmpty())
        m_status->setText(error);
    updateControls();
}

void PoliciesDialog::updateControls()
{
    const int count = m_rules->rowCount();
    const int row = m_rules->currentRow();
    m_tabs->setTabVisible(0, count > 0);
    m_tabs->setTabVisible(1, (count > 0) && (m_preview->rowCount() > 0));
    m_tabs->setTabVisible(2, m_journal->rowCount() > 0);
    m_empty->setVisible(count == 0);
    m_enabled->setVisible(count > 0);
    m_tabs->setVisible((count > 0) || (m_journal->rowCount() > 0));
    m_remove->setVisible((count > 0) && (m_tabs->currentIndex() == 0));
    m_up->setVisible((count > 1) && (m_tabs->currentIndex() == 0));
    m_down->setVisible((count > 1) && (m_tabs->currentIndex() == 0));
    m_remove->setEnabled(row >= 0);
    m_up->setEnabled(row > 0);
    m_down->setEnabled((row >= 0) && (row + 1 < count));
    bool hasDelete = false;
    for (int index = 0; index < count; ++index)
    {
        const auto *action = qobject_cast<QComboBox *>(m_rules->cellWidget(index, 6));
        hasDelete |= action && (action->currentData() == u"delete_data"_s);
    }
    m_deleteData->setVisible(hasDelete);
    const int selected = m_preview->currentRow();
    const QTableWidgetItem *permission = (selected >= 0) ? m_preview->item(selected, 4) : nullptr;
    m_accept->setEnabled(permission && permission->data(Qt::UserRole).toBool());
}
