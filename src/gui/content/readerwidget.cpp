/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "readerwidget.h"

#include <utility>

#include <QComboBox>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QPlainTextEdit>
#include <QPointer>
#include <QPushButton>
#include <QRegularExpression>
#include <QResizeEvent>
#include <QScrollBar>
#include <QSignalBlocker>
#include <QSpinBox>
#include <QStackedWidget>
#include <QStringList>
#include <QTextBrowser>
#include <QTextDocument>
#include <QTextOption>
#include <QTimer>
#include <QToolButton>
#include <QUrl>
#include <QVBoxLayout>

#include "base/content/client.h"
#include "base/global.h"
#include "base/preferences.h"
#include "gui/flowlayout.h"
#include "gui/uithememanager.h"
#include "readerimages.h"
#include "readersettingsdialog.h"

namespace
{
    class ReaderTitle final : public QLabel
    {
    public:
        explicit ReaderTitle(QWidget *parent)
            : QLabel(parent)
        {
            setMaximumWidth(220);
        }

    private:
        void paintEvent(QPaintEvent *) override
        {
            QPainter painter(this);
            painter.setPen(palette().windowText().color());
            painter.drawText(rect(), Qt::AlignVCenter,
                fontMetrics().elidedText(text(), Qt::ElideRight, width()));
        }
    };

    class ReaderTextBrowser final : public QTextBrowser
    {
    public:
        explicit ReaderTextBrowser(QWidget *parent)
            : QTextBrowser(parent)
        {
            setFrameShape(QFrame::NoFrame);
            setOpenLinks(false);
            document()->setDocumentMargin(24);
            connect(this, &QTextBrowser::anchorClicked, this, [this](const QUrl &url)
            {
                if (url.scheme().isEmpty() && url.hasFragment())
                    scrollToAnchor(url.fragment());
                else if ((url.scheme() == u"https") || (url.scheme() == u"http"))
                    QDesktopServices::openUrl(url);
            });
        }

    private:
        QVariant loadResource(int type, const QUrl &name) override
        {
            if (name.isLocalFile())
                return QTextBrowser::loadResource(type, name);
            return {};
        }

        void resizeEvent(QResizeEvent *event) override
        {
            const int margin = qMax(0, (width() - 900) / 2);
            setViewportMargins(margin, 0, margin, 0);
            QTextBrowser::resizeEvent(event);
        }
    };

    QString errorMessage(const QJsonValue &value)
    {
        const QString message = value.isString() ? value.toString() : value.toObject().value(u"message"_s).toString();
        return message.isEmpty() ? Content::ReaderWidget::tr("The content could not be loaded.") : message;
    }
}

namespace Content
{
    class GlossaryDialog final : public QDialog
    {
    public:
        GlossaryDialog(const QString &documentId, QWidget *parent)
            : QDialog(parent)
            , m_documentId {documentId}
        {
            setWindowTitle(ReaderWidget::tr("Title glossary"));
            resize(560, 400);
            auto *layout = new QVBoxLayout(this);
            m_editor = new QPlainTextEdit(this);
            m_editor->setObjectName(u"readerGlossaryEntries"_s);
            m_editor->setPlaceholderText(ReaderWidget::tr("遥 = Haruka\n蓮 = Ren"));
            layout->addWidget(m_editor, 1);
            m_status = new QLabel(this);
            m_status->setWordWrap(true);
            layout->addWidget(m_status);
            auto *buttons = new QDialogButtonBox(QDialogButtonBox::Save | QDialogButtonBox::Cancel, this);
            m_save = buttons->button(QDialogButtonBox::Save);
            m_retry = buttons->addButton(ReaderWidget::tr("Retry"), QDialogButtonBox::ActionRole);
            connect(m_retry, &QPushButton::clicked, this, &GlossaryDialog::load);
            connect(buttons, &QDialogButtonBox::accepted, this, &GlossaryDialog::save);
            connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
            layout->addWidget(buttons);
            connect(Client::instance(), &Client::eventReceived, this,
                [this](const qint64 id, const QString &event, const QJsonValue &data)
            {
                if ((m_request == 0) || (id != m_request)
                    || ((event != u"result") && (event != u"complete") && (event != u"error")))
                {
                    return;
                }
                m_request = 0;
                if (event == u"error")
                {
                    m_status->setText(errorMessage(data));
                    m_save->setEnabled(m_saving);
                    m_editor->setEnabled(m_saving);
                    m_retry->setVisible(!m_saving);
                }
                else if (m_saving)
                {
                    accept();
                }
                else
                {
                    QStringList lines;
                    const QJsonObject entries = data.toObject();
                    for (auto it = entries.begin(); it != entries.end(); ++it)
                        lines.append(it.key() + u" = "_s + it.value().toString());
                    m_editor->setPlainText(lines.join(u'\n'));
                    m_editor->setEnabled(true);
                    m_save->setEnabled(true);
                    m_status->clear();
                }
            });
            load();
        }

        ~GlossaryDialog() override
        {
            if (m_request != 0)
                Client::instance()->cancel(m_request);
        }

    private:
        void load()
        {
            m_saving = false;
            m_editor->setEnabled(false);
            m_save->setEnabled(false);
            m_retry->hide();
            m_status->setText(ReaderWidget::tr("Loading…"));
            m_request = Client::instance()->request(u"reader.glossary"_s, {{u"documentId"_s, m_documentId}});
        }

        void save()
        {
            QJsonObject entries;
            for (const QString &line : m_editor->toPlainText().split(u'\n'))
            {
                if (line.trimmed().isEmpty())
                    continue;
                const qsizetype separator = line.indexOf(u'=');
                const QString source = line.left(separator).trimmed();
                const QString target = line.mid(separator + 1).trimmed();
                if ((separator < 1) || source.isEmpty() || target.isEmpty())
                {
                    m_status->setText(ReaderWidget::tr("Write each term as: original = translation."));
                    return;
                }
                if (entries.contains(source))
                {
                    m_status->setText(ReaderWidget::tr("The term “%1” appears twice.").arg(source));
                    return;
                }
                entries.insert(source, target);
            }
            m_saving = true;
            m_save->setEnabled(false);
            m_editor->setEnabled(false);
            m_status->setText(ReaderWidget::tr("Saving…"));
            m_request = Client::instance()->request(u"reader.glossary"_s,
                {{u"documentId"_s, m_documentId}, {u"entries"_s, entries}});
        }

        QString m_documentId;
        QPlainTextEdit *m_editor = nullptr;
        QLabel *m_status = nullptr;
        QPushButton *m_save = nullptr;
        QPushButton *m_retry = nullptr;
        qint64 m_request = 0;
        bool m_saving = false;
    };
}

Content::ReaderWidget::ReaderWidget(QWidget *parent)
    : QWidget(parent)
{
    setObjectName(u"contentReader"_s);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(8);

    auto *header = new FlowLayout(0, 8, 6);
    auto *back = new QToolButton(this);
    back->setObjectName(u"readerBack"_s);
    back->setArrowType(Qt::LeftArrow);
    back->setToolTip(tr("Back"));
    connect(back, &QToolButton::clicked, this, [this]
    {
        savePosition();
        if (m_request != 0)
            Client::instance()->cancel(std::exchange(m_request, 0));
        if (m_translationRequest != 0)
            Client::instance()->cancel(std::exchange(m_translationRequest, 0));
        m_translationEnabled = false;
        updateControls();
        emit closed();
    });
    header->addWidget(back);
    m_title = new ReaderTitle(this);
    m_title->setText(tr("Reader"));
    m_title->setObjectName(u"readerTitle"_s);
    QFont titleFont = m_title->font();
    titleFont.setBold(true);
    m_title->setFont(titleFont);
    header->addWidget(m_title);
    m_previous = new QToolButton(this);
    m_previous->setObjectName(u"readerPrevious"_s);
    m_previous->setArrowType(Qt::LeftArrow);
    header->addWidget(m_previous);
    m_chapters = new QComboBox(this);
    m_chapters->setObjectName(u"readerChapters"_s);
    m_chapters->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    m_chapters->setMinimumContentsLength(8);
    m_chapters->setMaximumWidth(260);
    m_chapters->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    header->addWidget(m_chapters);
    m_next = new QToolButton(this);
    m_next->setObjectName(u"readerNext"_s);
    m_next->setArrowType(Qt::RightArrow);
    header->addWidget(m_next);
    m_pageNumber = new QLabel(this);
    m_pageNumber->setObjectName(u"readerPageNumber"_s);
    header->addWidget(m_pageNumber);
    connect(m_chapters, &QComboBox::activated, this, [this](const int index) { readChapter(index); });
    connect(m_previous, &QToolButton::clicked, this, [this] { turnPage(-1); });
    connect(m_next, &QToolButton::clicked, this, [this] { turnPage(1); });
    m_translate = new QPushButton(this);
    m_translate->setObjectName(u"readerTranslate"_s);
    connect(m_translate, &QPushButton::clicked, this, &ReaderWidget::translate);
    header->addWidget(m_translate);
    m_original = new QPushButton(tr("Original"), this);
    m_original->setObjectName(u"readerOriginal"_s);
    m_original->setCheckable(true);
    m_original->setToolTip(tr("Show original"));
    connect(m_original, &QPushButton::toggled, this, [this] { renderPage(scrollPosition(), m_images->imageIndex()); });
    header->addWidget(m_original);
    m_glossary = new QPushButton(tr("Glossary"), this);
    m_glossary->setObjectName(u"readerGlossary"_s);
    connect(m_glossary, &QPushButton::clicked, this, &ReaderWidget::editGlossary);
    header->addWidget(m_glossary);
    m_settings = new QToolButton(this);
    m_settings->setObjectName(u"readerSettings"_s);
    m_settings->setIcon(UIThemeManager::instance()->getIcon(u"configure"_s, u"preferences-system"_s));
    m_settings->setToolTip(tr("Reader settings"));
    connect(m_settings, &QToolButton::clicked, this, [this]
    {
        auto *dialog = findChild<ReaderSettingsDialog *>(QString(), Qt::FindDirectChildrenOnly);
        if (!dialog)
        {
            dialog = new ReaderSettingsDialog(this);
            connect(this, &ReaderWidget::closed, dialog, &QDialog::close);
        }
        dialog->show();
        dialog->raise();
        dialog->activateWindow();
    });
    header->addWidget(m_settings);
    m_textSizeLabel = new QLabel(tr("Text size"), this);
    header->addWidget(m_textSizeLabel);
    m_textSize = new QSpinBox(this);
    m_textSize->setObjectName(u"readerTextSize"_s);
    m_textSize->setRange(10, 36);
    m_textSize->setValue(16);
    m_textSizeLabel->setBuddy(m_textSize);
    header->addWidget(m_textSize);
    layout->addLayout(header);
    connect(m_textSize, &QSpinBox::valueChanged, this, [this]
    {
        const double offset = scrollPosition();
        QFont font = m_text->document()->defaultFont();
        font.setPointSize(m_textSize->value());
        m_restoringPosition = true;
        m_text->document()->setDefaultFont(font);
        restorePosition(offset);
    });

    auto *feedback = new QHBoxLayout;
    m_status = new QLabel(this);
    m_status->setObjectName(u"readerStatus"_s);
    m_status->setWordWrap(true);
    m_status->setTextInteractionFlags(Qt::TextSelectableByMouse);
    feedback->addWidget(m_status, 1);
    m_retry = new QPushButton(tr("Retry"), this);
    m_retry->setObjectName(u"readerRetry"_s);
    connect(m_retry, &QPushButton::clicked, this, [this] { startRequest(m_requestMethod, m_requestParams); });
    feedback->addWidget(m_retry);
    layout->addLayout(feedback);

    m_content = new QStackedWidget(this);
    m_empty = new QLabel(tr("Choose a book or manga to read."), this);
    m_empty->setAlignment(Qt::AlignCenter);
    m_empty->setWordWrap(true);
    m_content->addWidget(m_empty);
    m_text = new ReaderTextBrowser(this);
    m_text->setObjectName(u"readerText"_s);
    QFont textFont = m_text->document()->defaultFont();
    textFont.setPointSize(m_textSize->value());
    m_text->document()->setDefaultFont(textFont);
    m_content->addWidget(m_text);
    m_images = new ImagePages(this);
    m_images->setObjectName(u"readerImages"_s);
    m_images->setSettings(Preferences::instance()->readerSettings());
    connect(Preferences::instance(), &Preferences::readerSettingsChanged, this, [this]
    {
        m_images->setSettings(Preferences::instance()->readerSettings());
    });
    connect(m_images, &ImagePages::pageTurnRequested, this, &ReaderWidget::turnPage);
    connect(m_images, &ImagePages::correctionRequested, this, &ReaderWidget::correctRegion);
    m_content->addWidget(m_images);
    layout->addWidget(m_content, 1);

    m_positionTimer = new QTimer(this);
    m_positionTimer->setSingleShot(true);
    m_positionTimer->setInterval(800);
    connect(m_images, &ImagePages::imageIndexChanged, this, [this](const int imageIndex)
    {
        m_imagePositions.insert(m_page.value(u"id"_s).toString(), imageIndex);
        m_positionTimer->start();
        updateControls();
    });
    connect(m_positionTimer, &QTimer::timeout, this, [this]
    {
        savePosition();
        if (m_translationEnabled && (m_translationRequest == 0))
            startAhead();
    });
    connect(m_text->verticalScrollBar(), &QScrollBar::valueChanged, this, [this]
    {
        if (!m_restoringPosition && !m_page.isEmpty() && (m_request == 0) && !m_positionTimer->isActive())
            m_positionTimer->start();
        updateControls();
    });
    connect(Client::instance(), &Client::eventReceived, this, &ReaderWidget::handleEvent);
    showStatus({});
    updateControls();
}

Content::ReaderWidget::~ReaderWidget()
{
    savePosition();
    if (m_request != 0)
        Client::instance()->cancel(m_request);
    if (m_translationRequest != 0)
        Client::instance()->cancel(m_translationRequest);
}

void Content::ReaderWidget::open(const QJsonObject &descriptor)
{
    openDocument(u"reader.open"_s, descriptor);
}

void Content::ReaderWidget::openDocument(const QString &method, const QJsonObject &params)
{
    savePosition();
    m_document = {};
    m_page = {};
    m_translatedPage = {};
    m_imagePositions.clear();
    m_translationEnabled = false;
    m_chapterId.clear();
    m_chapters->clear();
    m_title->setText(params.value(u"title"_s).toString(tr("Reader")));
    m_content->setCurrentWidget(m_empty);
    startRequest(method, params);
}

void Content::ReaderWidget::open(const QJsonObject &work, const QJsonObject &variant, const QJsonObject &resolved)
{
    open({{u"work"_s, work}, {u"variant"_s, variant}, {u"resolved"_s, resolved},
        {u"title"_s, work.value(u"title"_s)}});
}

void Content::ReaderWidget::openLocal(const QString &path)
{
    openDocument(u"reader.local"_s, {{u"path"_s, path}});
}

bool Content::ReaderWidget::chooseFile()
{
    const QPointer<ReaderWidget> guard {this};
    const QString path = QFileDialog::getOpenFileName(this, tr("Open a book or comic"), {},
        tr("Books and comics (*.epub *.cbz *.zip *.html *.htm *.txt *.fb2 *.png *.jpg *.jpeg *.webp *.gif *.bmp);;All files (*)"));
    if (!guard || path.isEmpty())
        return false;

    openLocal(path);
    return true;
}

void Content::ReaderWidget::startRequest(const QString &method, const QJsonObject &params)
{
    if (m_request != 0)
        Client::instance()->cancel(m_request);
    if (m_translationRequest != 0)
    {
        Client::instance()->cancel(m_translationRequest);
        m_translationRequest = 0;
    }
    m_requestMethod = method;
    m_requestParams = params;
    m_request = Client::instance()->request(m_requestMethod, m_requestParams);
    showStatus(method == u"reader.chapter" ? tr("Loading chapter…") : tr("Opening…"));
    updateControls();
}

void Content::ReaderWidget::handleEvent(const qint64 id, const QString &event, const QJsonValue &data)
{
    if ((id == 0) || ((id != m_request) && (id != m_translationRequest)))
        return;
    if (event == u"page")
    {
        const QJsonObject page = data.toObject();
        if ((id == m_request) && (m_requestMethod == u"reader.chapter"))
        {
            m_chapterId = page.value(u"id"_s).toString();
            setPage(page);
            updateControls();
        }
        else if ((id == m_translationRequest) && page.value(u"translated"_s).toBool()
            && (page.value(u"id"_s) == m_page.value(u"id"_s)) && (page != m_translatedPage))
        {
            m_translatedPage = page;
            renderPage(scrollPosition(), m_images->imageIndex());
        }
        return;
    }
    if (event == u"progress")
    {
        const QJsonObject progress = data.toObject();
        QString message = progress.value(u"message"_s).toString();
        if (message.isEmpty())
        {
            const QString phase = progress.value(u"phase"_s).toString();
            if (phase == u"connecting")
                message = tr("Connecting to the translation service…");
            else if (phase == u"recognizing")
                message = tr("Recognizing text…");
            else if (phase == u"cleaning")
                message = tr("Preparing translated page…");
            else if (phase == u"translating")
                message = tr("Translating…");
        }
        if (progress.value(u"total"_s).toInt() > 0)
        {
            message += tr(": %1 of %2").arg(progress.value(u"completed"_s).toInt())
                .arg(progress.value(u"total"_s).toInt());
        }
        if (!message.isEmpty())
            showStatus(message);
        return;
    }
    if ((event != u"result") && (event != u"complete") && (event != u"error"))
        return;

    if (id == m_translationRequest)
    {
        m_translationRequest = 0;
        if (event == u"error")
        {
            m_translationEnabled = false;
            showStatus(errorMessage(data));
        }
        else if (data.toObject().value(u"id"_s) == m_page.value(u"id"_s))
        {
            m_translatedPage = data.toObject();
            const QSignalBlocker blocker(m_original);
            m_original->setChecked(false);
            renderPage(scrollPosition(), m_images->imageIndex());
            showStatus({});
            QTimer::singleShot(0, this, &ReaderWidget::startAhead);
        }
        else
        {
            showStatus({});
        }
    }
    else
    {
        m_request = 0;
        if (event == u"error")
        {
            const QSignalBlocker blocker(m_chapters);
            m_chapters->setCurrentIndex(m_chapters->findData(m_chapterId));
            showStatus(errorMessage(data), true);
        }
        else if (m_requestMethod == u"reader.chapter")
        {
            if (m_page.value(u"id"_s) == data.toObject().value(u"id"_s))
            {
                m_pendingOffset = scrollPosition();
                m_pendingImageIndex = m_images->imageIndex();
            }
            m_chapterId = m_requestParams.value(u"chapter"_s).toObject().value(u"id"_s).toString();
            m_chapters->setCurrentIndex(m_chapters->findData(m_chapterId));
            setPage(data.toObject());
        }
        else
        {
            setDocument(data.toObject());
        }
    }
    updateControls();
}

void Content::ReaderWidget::setDocument(const QJsonObject &document)
{
    m_document = document;
    const QString source = document.value(u"sourceId"_s).toString();
    m_translationEnabled = (source == u"kakuyomu") || (source == u"syosetu") || (source == u"royalroad") || (source == u"linovel");
    m_title->setText(document.value(u"title"_s).toString());
    m_title->setToolTip(m_title->text());
    const QSignalBlocker blocker(m_chapters);
    m_chapters->clear();
    const QJsonArray chapters = document.value(u"chapters"_s).toArray();
    for (const QJsonValue &value : chapters)
    {
        const QJsonObject chapter = value.toObject();
        m_chapters->addItem(chapter.value(u"title"_s).toString(), chapter.value(u"id"_s).toString());
    }
    if (chapters.isEmpty())
    {
        showStatus(tr("This title has no available chapters yet."));
        return;
    }

    const QJsonObject position = document.value(u"position"_s).toObject();
    const int savedChapter = m_chapters->findData(position.value(u"chapterId"_s).toString());
    m_pendingOffset = qBound(0.0, position.value(u"offset"_s).toDouble(), 1.0);
    m_imagePositions.insert(position.value(u"chapterId"_s).toString(), qMax(0, position.value(u"imageIndex"_s).toInt()));
    readChapter(qMax(0, savedChapter));
}

void Content::ReaderWidget::readChapter(const int index, const std::optional<int> imageIndex)
{
    const QJsonArray chapters = m_document.value(u"chapters"_s).toArray();
    if ((index < 0) || (index >= chapters.size()))
        return;

    savePosition();
    if (!m_page.isEmpty())
        m_pendingOffset = 0;
    const QString chapterId = chapters[index].toObject().value(u"id"_s).toString();
    m_pendingImageIndex = imageIndex.value_or(m_imagePositions.value(chapterId, 0));
    m_chapters->setCurrentIndex(index);
    startRequest(u"reader.chapter"_s, {{u"sourceId"_s, m_document.value(u"sourceId"_s)},
        {u"documentId"_s, m_document.value(u"id"_s)}, {u"chapter"_s, chapters[index]}});
}

void Content::ReaderWidget::turnPage(const int delta)
{
    if ((m_request != 0) || m_page.isEmpty())
        return;

    if (m_page.value(u"kind"_s).toString() == u"images")
    {
        const int imageIndex = m_images->imageIndex() + delta;
        const qsizetype count = m_page.value(u"images"_s).toArray().size();
        if ((imageIndex >= 0) && (imageIndex < count))
        {
            renderPage(0, imageIndex);
            savePosition();
            startAhead();
        }
        else
        {
            // Enter the preceding chapter at its last image, or the following one at its first.
            readChapter(m_chapters->currentIndex() + delta, delta < 0 ? -1 : 0);
        }
    }
    else
    {
        readChapter(m_chapters->currentIndex() + delta);
    }
}

void Content::ReaderWidget::setPage(const QJsonObject &page)
{
    const bool firstPage = m_page.isEmpty();
    m_page = page;
    m_translatedPage = {};
    const QSignalBlocker blocker(m_original);
    m_original->setChecked(false);
    renderPage(m_pendingOffset, m_pendingImageIndex);
    showStatus({});
    if (firstPage && (page.value(u"kind"_s).toString() == u"images"))
        m_images->setFocus(Qt::OtherFocusReason);
    if (firstPage && (page.value(u"kind"_s).toString() == u"text"))
    {
        const QString text = page.value(u"html"_s).toString();
        const QRegularExpression asian(u"[\\x{3040}-\\x{30ff}\\x{3400}-\\x{9fff}\\x{ac00}-\\x{d7af}]"_s);
        auto characters = asian.globalMatch(text);
        int count = 0;
        while (characters.hasNext())
        {
            characters.next();
            ++count;
        }
        m_translationEnabled = m_translationEnabled || (count > 10);
    }
    if (m_translationEnabled)
        QTimer::singleShot(0, this, &ReaderWidget::startAhead);
}

void Content::ReaderWidget::renderPage(const double offset, const int imageIndex)
{
    if (m_page.isEmpty())
        return;

    const QJsonObject page = (!m_translatedPage.isEmpty() && !m_original->isChecked()) ? m_translatedPage : m_page;
    if (page.value(u"kind"_s).toString() == u"images")
    {
        m_images->setPage(page, imageIndex, m_original->isChecked());
        m_imagePositions.insert(m_page.value(u"id"_s).toString(), m_images->imageIndex());
        m_content->setCurrentWidget(m_images);
    }
    else
    {
        m_restoringPosition = true;
        QTextOption option = m_text->document()->defaultTextOption();
        option.setTextDirection(page.value(u"direction"_s).toString() == u"rtl" ? Qt::RightToLeft : Qt::LeftToRight);
        m_text->document()->setDefaultTextOption(option);
        m_text->setHtml(page.value(u"html"_s).toString());
        m_content->setCurrentWidget(m_text);
        restorePosition(offset);
    }
    updateControls();
}

void Content::ReaderWidget::translate()
{
    if (m_translationEnabled || (m_translationRequest != 0))
    {
        m_translationEnabled = false;
        if (m_translationRequest != 0)
            Client::instance()->cancel(std::exchange(m_translationRequest, 0));
        showStatus(tr("Translation cancelled."));
        updateControls();
        return;
    }
    if (m_page.isEmpty())
        return;
    m_translationEnabled = true;
    startAhead();
}

void Content::ReaderWidget::startAhead()
{
    if (!m_translationEnabled || m_page.isEmpty() || (m_request != 0) || (m_translationRequest != 0))
        return;

    QJsonObject params {{u"sourceId"_s, m_document.value(u"sourceId"_s)},
        {u"documentId"_s, m_document.value(u"id"_s)}, {u"chapterId"_s, m_page.value(u"id"_s)},
        {u"targetLanguage"_s, u"ru"_s}};
    if (m_page.value(u"kind"_s).toString() == u"images")
    {
        params.insert(u"imageIndex"_s, m_images->imageIndex());
    }
    else
    {
        params.insert(u"offset"_s, scrollPosition());
    }
    m_translationRequest = Client::instance()->request(u"reader.ahead"_s, params);
    showStatus({});
    updateControls();
}

void Content::ReaderWidget::correctRegion(const QJsonObject &region)
{
    const bool resume = m_translationEnabled;
    m_translationEnabled = false;
    if (m_translationRequest != 0)
        Client::instance()->cancel(std::exchange(m_translationRequest, 0));

    bool accepted = false;
    const QPointer<ReaderWidget> guard {this};
    const QString text = QInputDialog::getMultiLineText(this, tr("Correct translation"),
        tr("Original: %1").arg(region.value(u"text"_s).toString()),
        region.value(u"translation"_s).toString(), &accepted);
    if (!guard)
        return;

    m_translationEnabled = resume;
    if (!accepted || (text == region.value(u"translation"_s).toString()))
    {
        if (m_translationEnabled)
            startAhead();
        return;
    }

    m_translationRequest = Client::instance()->request(u"reader.correct"_s,
        {{u"documentId"_s, m_document.value(u"id"_s)}, {u"pageId"_s, m_page.value(u"id"_s)},
        {u"regionId"_s, region.value(u"id"_s)}, {u"text"_s, text}});
    updateControls();
}

void Content::ReaderWidget::editGlossary()
{
    const bool resume = m_translationEnabled;
    m_translationEnabled = false;
    if (m_translationRequest != 0)
        Client::instance()->cancel(std::exchange(m_translationRequest, 0));
    updateControls();
    auto *dialog = new GlossaryDialog(m_document.value(u"id"_s).toString(), this);
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    connect(dialog, &QDialog::finished, this, [this, resume](const int result)
    {
        if (result == QDialog::Accepted)
        {
            const double offset = scrollPosition();
            m_translatedPage = {};
            const QSignalBlocker blocker(m_original);
            m_original->setChecked(false);
            renderPage(offset, m_images->imageIndex());
            showStatus({});
        }
        m_translationEnabled = resume;
        if (m_translationEnabled)
            startAhead();
        updateControls();
    });
    dialog->open();
}

void Content::ReaderWidget::updateControls()
{
    const bool ready = (m_request == 0);
    const bool hasPage = !m_page.isEmpty();
    const bool images = m_page.value(u"kind"_s).toString() == u"images";
    const bool hasChapters = (m_chapters->count() > 0);
    m_chapters->setVisible(hasChapters);
    m_previous->setVisible(hasChapters);
    m_next->setVisible(hasChapters);
    const int imageCount = static_cast<int>(m_page.value(u"images"_s).toArray().size());
    m_previous->setEnabled(ready && ((m_chapters->currentIndex() > 0) || (images && (m_images->imageIndex() > 0))));
    m_next->setEnabled(ready && ((m_chapters->currentIndex() + 1 < m_chapters->count())
        || (images && (m_images->imageIndex() + 1 < imageCount))));
    m_previous->setToolTip(images ? tr("Previous page") : tr("Previous chapter"));
    m_next->setToolTip(images ? tr("Next page") : tr("Next chapter"));
    m_pageNumber->setVisible(hasPage && images);
    m_settings->setVisible(hasPage && images);
    m_pageNumber->setText(tr("%1 / %2").arg(qMin(m_images->imageIndex() + 1, imageCount)).arg(imageCount));
    m_images->setContextMenuPolicy(ready ? Qt::DefaultContextMenu : Qt::PreventContextMenu);
    m_translate->setVisible(hasPage);
    m_original->setVisible(!m_translatedPage.isEmpty());
    m_original->setEnabled(ready);
    m_glossary->setVisible(hasPage);
    m_glossary->setEnabled(ready);
    m_textSize->setVisible(hasPage && !images);
    m_textSizeLabel->setVisible(hasPage && !images);

    m_translate->setEnabled(ready && hasPage);
    m_translate->setText(m_translationEnabled ? tr("Stop translation")
        : images ? tr("Translate pages") : tr("Translate chapters"));
}

void Content::ReaderWidget::showStatus(const QString &message, const bool canRetry)
{
    m_status->setText(message);
    m_status->setVisible(!message.isEmpty());
    m_retry->setVisible(canRetry);
}

double Content::ReaderWidget::scrollPosition() const
{
    const QScrollBar *bar = m_text->verticalScrollBar();
    return (bar->maximum() > 0) ? double(bar->value()) / bar->maximum() : 0;
}

void Content::ReaderWidget::restorePosition(const double offset)
{
    QTimer::singleShot(0, this, [this, offset]
    {
        QScrollBar *bar = m_text->verticalScrollBar();
        bar->setValue(qRound(qBound(0.0, offset, 1.0) * bar->maximum()));
        m_restoringPosition = false;
    });
}

void Content::ReaderWidget::savePosition()
{
    m_positionTimer->stop();
    if (m_document.isEmpty() || m_chapterId.isEmpty() || m_page.isEmpty() || m_restoringPosition)
        return;

    QJsonObject progress {{u"documentId"_s, m_document.value(u"id"_s)},
        {u"workId"_s, m_document.value(u"workId"_s)},
        {u"chapterId"_s, m_chapterId}};
    if (m_page.value(u"kind"_s).toString() == u"images")
    {
        progress.insert(u"imageIndex"_s, m_images->imageIndex());
    }
    else
    {
        progress.insert(u"offset"_s, scrollPosition());
    }
    Client::instance()->request(u"reader.position"_s, progress);
    emit progressChanged(progress);
}
