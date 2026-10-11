// SPDX-License-Identifier: GPL-2.0-or-later
#include "downloadswidget.h"

#include <QDesktopServices>
#include <QFileInfo>
#include <QHeaderView>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPushButton>
#include <QStandardItemModel>
#include <QTableView>
#include <QUrl>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"

Content::DownloadsWidget::DownloadsWidget(QWidget *parent)
    : QWidget(parent)
{
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    m_model = new QStandardItemModel(this);
    m_model->setHorizontalHeaderLabels({tr("File"), tr("Status"), tr("Progress")});
    m_view = new QTableView(this);
    m_view->setObjectName(u"contentDownloads"_s);
    m_view->setModel(m_model);
    m_view->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_view->setSelectionMode(QAbstractItemView::SingleSelection);
    m_view->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_view->setShowGrid(false);
    m_view->setWordWrap(false);
    m_view->verticalHeader()->hide();
    m_view->verticalHeader()->setDefaultSectionSize(34);
    m_view->horizontalHeader()->setHighlightSections(false);
    m_view->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
    m_view->horizontalHeader()->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    m_view->horizontalHeader()->setSectionResizeMode(2, QHeaderView::ResizeToContents);
    layout->addWidget(m_view, 1);
    auto *actions = new QHBoxLayout;
    m_openFolder = new QPushButton(tr("Open folder"), this);
    m_stop = new QPushButton(tr("Stop download"), this);
    actions->addWidget(m_openFolder);
    actions->addWidget(m_stop);
    m_status = new QLabel(this);
    m_status->setWordWrap(true);
    actions->addWidget(m_status, 1);
    layout->addLayout(actions);

    connect(m_view->selectionModel(), &QItemSelectionModel::currentRowChanged, this, &DownloadsWidget::updateActions);
    connect(m_model, &QStandardItemModel::dataChanged, this, &DownloadsWidget::updateActions);
    connect(m_openFolder, &QPushButton::clicked, this, [this]
    {
        const QString path = selectedJob().value(u"path"_s).toString();
        if (!path.isEmpty())
            QDesktopServices::openUrl(QUrl::fromLocalFile(QFileInfo(path).absolutePath()));
    });
    connect(m_stop, &QPushButton::clicked, this, [this]
    {
        m_requests.insert(Client::instance()->request(u"cancelDownload"_s, {{u"id"_s, selectedJob().value(u"id"_s)}}),
            tr("Download stopped"));
    });
    connect(Client::instance(), &Client::eventReceived, this, &DownloadsWidget::handleEvent);
    updateActions();
    refresh();
}

void Content::DownloadsWidget::enqueue(const QJsonObject &params)
{
    m_requests.insert(Client::instance()->request(u"download"_s, params),
        tr("Download queued"));
    emit preparationChanged();
    emit statusChanged(tr("Preparing download…"));
}

void Content::DownloadsWidget::refresh()
{
    m_jobsRequest = Client::instance()->request(u"jobs"_s);
}

bool Content::DownloadsWidget::isPreparing() const
{
    return !m_requests.isEmpty();
}

QJsonObject Content::DownloadsWidget::selectedJob() const
{
    return m_view->currentIndex().siblingAtColumn(0).data(Qt::UserRole).value<QJsonObject>();
}

void Content::DownloadsWidget::updateActions()
{
    const QJsonObject job = selectedJob();
    m_openFolder->setEnabled(!job.value(u"path"_s).toString().isEmpty());
    const QString status = job.value(u"status"_s).toString();
    m_stop->setEnabled((status == u"queued") || (status == u"downloading"));
    m_status->setText(job.value(u"message"_s).toString());
}

void Content::DownloadsWidget::updateJob(const QJsonObject &job)
{
    const QString id = job.value(u"id"_s).toString();
    if (id.isEmpty())
        return;

    int row = m_rows.value(id, -1);
    if (row < 0)
    {
        row = m_model->rowCount();
        m_rows.insert(id, row);
        m_model->appendRow({new QStandardItem, new QStandardItem, new QStandardItem});
    }
    QJsonObject snapshot = m_model->item(row, 0)->data(Qt::UserRole).value<QJsonObject>();
    for (auto it = job.constBegin(); it != job.constEnd(); ++it)
        snapshot.insert(it.key(), it.value());
    m_model->item(row, 0)->setData(snapshot, Qt::UserRole);
    QString title = snapshot.value(u"title"_s).toString();
    if (title.isEmpty())
        title = QFileInfo(snapshot.value(u"path"_s).toString()).fileName();
    m_model->item(row, 0)->setText(title);
    const QString status = snapshot.value(u"status"_s).toString();
    QString statusText;
    if (status == u"queued")
        statusText = tr("Queued");
    else if (status == u"downloading")
        statusText = tr("Downloading");
    else if (status == u"complete")
        statusText = tr("Complete");
    else if (status == u"cancelled")
        statusText = tr("Stopped");
    else if (status == u"error")
        statusText = tr("Failed");
    m_model->item(row, 1)->setText(statusText);
    m_model->item(row, 1)->setToolTip(snapshot.value(u"message"_s).toString());
    const QJsonValue progress = snapshot.value(u"progress"_s);
    m_model->item(row, 2)->setText(progress.isDouble() ? tr("%1%").arg(qRound(progress.toDouble() * 100)) : QString());
}

void Content::DownloadsWidget::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if (((id == 0) || m_requests.contains(id)) && ((event == u"job") || (event == u"progress")))
    {
        updateJob(data.toObject());
    }
    else if (m_requests.contains(id) && ((event == u"complete") || (event == u"error")))
    {
        const QString successMessage = m_requests.take(id);
        const QString message = (event == u"error") ? data.toObject().value(u"message"_s).toString() : successMessage;
        m_status->setText(message);
        emit statusChanged(message);
        emit preparationChanged();
    }
    else if (id == m_jobsRequest)
    {
        if (event == u"jobs")
        {
            for (const QJsonValue &value : data.toArray())
                updateJob(value.toObject());
        }
        else if (event == u"error")
        {
            m_status->setText(data.toObject().value(u"message"_s).toString());
        }
    }
}
