// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QDialog>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>

class QCheckBox;
class QFormLayout;
class QLabel;
class QLineEdit;
class QListWidget;
class QPushButton;

namespace Content
{
    class BrowserConnectionWidget;

    class SourcesDialog final : public QDialog
    {
        Q_OBJECT

    public:
        explicit SourcesDialog(QWidget *parent = nullptr, const QString &sourceId = {});
        ~SourcesDialog() override;

    signals:
        void publicationInspected(const QJsonObject &details);

    private:
        void filterSources();
        void selectSource();
        void saveSource();
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);

        QListWidget *m_sources = nullptr;
        QLineEdit *m_filter = nullptr;
        QLabel *m_status = nullptr;
        QLabel *m_name = nullptr;
        QLabel *m_health = nullptr;
        BrowserConnectionWidget *m_browserConnection = nullptr;
        QCheckBox *m_enabled = nullptr;
        QFormLayout *m_form = nullptr;
        QPushButton *m_save = nullptr;
        QPushButton *m_check = nullptr;
        QPushButton *m_refresh = nullptr;
        QHash<QString, QWidget *> m_fields;
        QJsonArray m_catalog;
        QString m_initialSource;
        qint64 m_request = 0;
    };
}
