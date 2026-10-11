/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <optional>

#include <QHash>
#include <QJsonObject>
#include <QWidget>

class QComboBox;
class QLabel;
class QPushButton;
class QSpinBox;
class QStackedWidget;
class QTextBrowser;
class QTimer;
class QToolButton;

namespace Content
{
    class ImagePages;

    class ReaderWidget final : public QWidget
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(ReaderWidget)

    public:
        explicit ReaderWidget(QWidget *parent = nullptr);
        ~ReaderWidget() override;

        void open(const QJsonObject &descriptor);
        void open(const QJsonObject &work, const QJsonObject &variant, const QJsonObject &resolved);
        void openLocal(const QString &path);

    public slots:
        bool chooseFile();

    signals:
        void closed();
        void progressChanged(const QJsonObject &progress);

    private:
        void openDocument(const QString &method, const QJsonObject &params);
        void startRequest(const QString &method, const QJsonObject &params);
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);
        void setDocument(const QJsonObject &document);
        void readChapter(int index, std::optional<int> imageIndex = std::nullopt);
        void turnPage(int delta);
        void setPage(const QJsonObject &page);
        void renderPage(double offset, int imageIndex);
        void translate();
        void startAhead();
        void editGlossary();
        void correctRegion(const QJsonObject &region);
        void updateControls();
        void showStatus(const QString &message, bool canRetry = false);
        void savePosition();
        double scrollPosition() const;
        void restorePosition(double offset);

        QLabel *m_title = nullptr;
        QComboBox *m_chapters = nullptr;
        QToolButton *m_previous = nullptr;
        QToolButton *m_next = nullptr;
        QToolButton *m_settings = nullptr;
        QLabel *m_pageNumber = nullptr;
        QPushButton *m_translate = nullptr;
        QPushButton *m_original = nullptr;
        QPushButton *m_glossary = nullptr;
        QSpinBox *m_textSize = nullptr;
        QLabel *m_textSizeLabel = nullptr;
        QLabel *m_status = nullptr;
        QPushButton *m_retry = nullptr;
        QStackedWidget *m_content = nullptr;
        QTextBrowser *m_text = nullptr;
        ImagePages *m_images = nullptr;
        QLabel *m_empty = nullptr;
        QTimer *m_positionTimer = nullptr;

        QJsonObject m_document;
        QJsonObject m_page;
        QJsonObject m_translatedPage;
        QHash<QString, int> m_imagePositions;
        QString m_chapterId;
        QString m_requestMethod;
        QJsonObject m_requestParams;
        qint64 m_request = 0;
        qint64 m_translationRequest = 0;
        double m_pendingOffset = 0;
        int m_pendingImageIndex = 0;
        bool m_restoringPosition = false;
        bool m_translationEnabled = false;
    };
}
