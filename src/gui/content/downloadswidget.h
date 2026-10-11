// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QJsonObject>
#include <QWidget>

class QLabel;
class QPushButton;
class QStandardItemModel;
class QTableView;

namespace Content
{
    class DownloadsWidget final : public QWidget
    {
        Q_OBJECT

    public:
        explicit DownloadsWidget(QWidget *parent = nullptr);
        void enqueue(const QJsonObject &params);
        void refresh();
        bool isPreparing() const;

    signals:
        void statusChanged(const QString &message);
        void preparationChanged();

    private:
        void updateJob(const QJsonObject &job);
        void updateActions();
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);
        QJsonObject selectedJob() const;

        QTableView *m_view = nullptr;
        QStandardItemModel *m_model = nullptr;
        QLabel *m_status = nullptr;
        QPushButton *m_openFolder = nullptr;
        QPushButton *m_stop = nullptr;
        QHash<QString, int> m_rows;
        QHash<qint64, QString> m_requests;
        qint64 m_jobsRequest = 0;
    };
}
