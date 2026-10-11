// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QDialog>
#include <QHash>
#include <QJsonArray>

class QCheckBox;
class QFormLayout;
class QLabel;
class QLineEdit;
class QListWidget;
class QPushButton;

namespace Content
{
    class SourcesDialog final : public QDialog
    {
        Q_OBJECT

    public:
        explicit SourcesDialog(QWidget *parent = nullptr);

    protected:
        void changeEvent(QEvent *event) override;

    private:
        void loadSources();
        void filterSources();
        void selectSource();
        void saveSource();
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);

        QListWidget *m_sources = nullptr;
        QLineEdit *m_filter = nullptr;
        QLabel *m_status = nullptr;
        QLabel *m_name = nullptr;
        QLabel *m_browserSetup = nullptr;
        QLineEdit *m_extensionDirectory = nullptr;
        QCheckBox *m_enabled = nullptr;
        QFormLayout *m_form = nullptr;
        QPushButton *m_save = nullptr;
        QPushButton *m_refresh = nullptr;
        QPushButton *m_connect = nullptr;
        QHash<QString, QWidget *> m_fields;
        QJsonArray m_catalog;
        qint64 m_request = 0;
        qint64 m_connectRequest = 0;
    };
}
