package app.still.browser

import android.app.AlertDialog
import android.app.DatePickerDialog
import android.app.TimePickerDialog
import android.content.Intent
import android.text.InputType
import android.widget.EditText
import android.widget.LinearLayout
import org.mozilla.geckoview.AllowOrDeny
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoSession.PromptDelegate
import org.mozilla.geckoview.GeckoSession.PromptDelegate.AlertPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.AuthPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.BasePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.BeforeUnloadPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.ButtonPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.ChoicePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.ColorPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.DateTimePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.FilePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.PopupPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.PromptResponse
import org.mozilla.geckoview.GeckoSession.PromptDelegate.RepostConfirmPrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.SharePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.TextPrompt
import java.util.Calendar

/**
 * Dialogs a page asks for: alert/confirm/prompt, `<select>` menus, date and
 * colour pickers, HTTP sign-in, leaving a page, file uploads and sharing.
 * GeckoView draws none of these itself. Each answer completes the page's
 * request exactly once; a dialog closed without an answer dismisses it.
 */
class Prompts(private val activity: MainActivity) : PromptDelegate {

    private fun <P : BasePrompt> ask(prompt: P, build: AlertDialog.Builder.(answer: (PromptResponse) -> Unit) -> Unit): GeckoResult<PromptResponse> {
        val result = GeckoResult<PromptResponse>()
        if (activity.isFinishing || activity.isDestroyed) return GeckoResult.fromValue(prompt.dismiss())
        var answered = false
        val answer = { response: PromptResponse ->
            if (!answered) {
                answered = true
                result.complete(response)
            }
        }
        val builder = AlertDialog.Builder(activity)
        prompt.title?.takeIf { it.isNotBlank() }?.let(builder::setTitle)
        builder.build(answer)
        val dialog = builder.setOnDismissListener { answer(prompt.dismiss()) }.create()
        // The page can withdraw its request (it navigated away, or timed out).
        prompt.delegate = object : PromptDelegate.PromptInstanceDelegate {
            override fun onPromptDismiss(prompt: BasePrompt) {
                answered = true
                dialog.dismiss()
            }
        }
        dialog.show()
        return result
    }

    private fun field(text: String?, type: Int = InputType.TYPE_CLASS_TEXT) = EditText(activity).apply {
        inputType = type
        background = rounded(activity.palette.field, activity.dp(8))
        setTextColor(activity.palette.label)
        setHintTextColor(activity.palette.secondary)
        setPadding(activity.dp(12), activity.dp(12), activity.dp(12), activity.dp(12))
        setText(text.orEmpty())
        setSelection(this.text.length)
    }

    private fun padded(vararg fields: EditText) = LinearLayout(activity).apply {
        orientation = LinearLayout.VERTICAL
        val side = activity.dp(20)
        setPadding(side, activity.dp(8), side, 0)
        fields.forEach { addView(it) }
    }

    override fun onAlertPrompt(session: GeckoSession, prompt: AlertPrompt) = ask(prompt) { answer ->
        setMessage(prompt.message)
        setPositiveButton(android.R.string.ok) { _, _ -> answer(prompt.dismiss()) }
    }

    override fun onButtonPrompt(session: GeckoSession, prompt: ButtonPrompt) = ask(prompt) { answer ->
        setMessage(prompt.message)
        setPositiveButton(android.R.string.ok) { _, _ -> answer(prompt.confirm(ButtonPrompt.Type.POSITIVE)) }
        setNegativeButton(android.R.string.cancel) { _, _ -> answer(prompt.confirm(ButtonPrompt.Type.NEGATIVE)) }
    }

    override fun onTextPrompt(session: GeckoSession, prompt: TextPrompt) = ask(prompt) { answer ->
        val input = field(prompt.defaultValue)
        setMessage(prompt.message)
        setView(padded(input))
        setPositiveButton(android.R.string.ok) { _, _ -> answer(prompt.confirm(input.text.toString())) }
        setNegativeButton(android.R.string.cancel, null)
    }

    override fun onAuthPrompt(session: GeckoSession, prompt: AuthPrompt) = ask(prompt) { answer ->
        val onlyPassword = prompt.authOptions.flags and AuthPrompt.AuthOptions.Flags.ONLY_PASSWORD != 0
        val user = field(prompt.authOptions.username).apply { hint = activity.getString(R.string.prompt_username) }
        val password = field(prompt.authOptions.password, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
            .apply { hint = activity.getString(R.string.prompt_password) }
        setMessage(prompt.message)
        setView(if (onlyPassword) padded(password) else padded(user, password))
        setPositiveButton(R.string.prompt_sign_in) { _, _ ->
            answer(if (onlyPassword) prompt.confirm(password.text.toString()) else prompt.confirm(user.text.toString(), password.text.toString()))
        }
        setNegativeButton(android.R.string.cancel, null)
    }

    /** `<select>` and `<select multiple>`; option groups are shown flattened. */
    override fun onChoicePrompt(session: GeckoSession, prompt: ChoicePrompt) = ask(prompt) { answer ->
        val options = ArrayList<Pair<ChoicePrompt.Choice, String>>()
        fun collect(choices: Array<ChoicePrompt.Choice>, group: String?) {
            for (choice in choices) {
                when {
                    choice.items != null -> collect(choice.items!!, choice.label)
                    choice.separator || choice.disabled -> Unit
                    else -> options += choice to (group?.let { "$it › ${choice.label}" } ?: choice.label)
                }
            }
        }
        collect(prompt.choices, null)
        val labels = options.map { it.second }.toTypedArray()
        prompt.message?.takeIf { it.isNotBlank() && prompt.title.isNullOrBlank() }?.let(::setTitle)
        if (prompt.type == ChoicePrompt.Type.MULTIPLE) {
            val checked = BooleanArray(options.size) { options[it].first.selected }
            setMultiChoiceItems(labels, checked) { _, which, isChecked -> checked[which] = isChecked }
            setPositiveButton(android.R.string.ok) { _, _ ->
                answer(prompt.confirm(options.filterIndexed { i, _ -> checked[i] }.map { it.first }.toTypedArray()))
            }
            setNegativeButton(android.R.string.cancel, null)
        } else {
            val selected = options.indexOfFirst { it.first.selected }
            setSingleChoiceItems(labels, selected) { dialog, which ->
                answer(prompt.confirm(options[which].first))
                dialog.dismiss()
            }
        }
    }

    override fun onColorPrompt(session: GeckoSession, prompt: ColorPrompt) = ask(prompt) { answer ->
        val input = field(prompt.defaultValue ?: "#000000")
        input.hint = "#rrggbb"
        setView(padded(input))
        setPositiveButton(android.R.string.ok) { _, _ ->
            val value = input.text.toString().trim()
            answer(if (Regex("^#[0-9a-fA-F]{6}$").matches(value)) prompt.confirm(value.lowercase()) else prompt.dismiss())
        }
        setNegativeButton(android.R.string.cancel, null)
    }

    /**
     * `<input type=date|time|datetime-local>` use the system pickers; month and
     * week inputs take text in the page's format (2026-10, 2026-W42).
     */
    override fun onDateTimePrompt(session: GeckoSession, prompt: DateTimePrompt): GeckoResult<PromptResponse> {
        val type = prompt.type
        if (type == DateTimePrompt.Type.MONTH || type == DateTimePrompt.Type.WEEK) {
            return ask(prompt) { answer ->
                val input = field(prompt.defaultValue)
                input.hint = if (type == DateTimePrompt.Type.MONTH) "2026-10" else "2026-W42"
                setView(padded(input))
                setPositiveButton(android.R.string.ok) { _, _ -> answer(prompt.confirm(input.text.toString().trim())) }
                setNeutralButton(R.string.prompt_clear) { _, _ -> answer(prompt.confirm("")) }
                setNegativeButton(android.R.string.cancel, null)
            }
        }
        val result = GeckoResult<PromptResponse>()
        var answered = false
        val finish = { response: PromptResponse -> if (!answered) { answered = true; result.complete(response) } }
        val now = Calendar.getInstance()
        val value = prompt.defaultValue.orEmpty()
        val date = Regex("^(\\d{4})-(\\d{2})-(\\d{2})").find(value)?.groupValues
        val time = Regex("(?:^|T)(\\d{2}):(\\d{2})").find(value)?.groupValues
        val year = date?.get(1)?.toInt() ?: now.get(Calendar.YEAR)
        val month = date?.get(2)?.toInt()?.minus(1) ?: now.get(Calendar.MONTH)
        val day = date?.get(3)?.toInt() ?: now.get(Calendar.DAY_OF_MONTH)
        val hour = time?.get(1)?.toInt() ?: now.get(Calendar.HOUR_OF_DAY)
        val minute = time?.get(2)?.toInt() ?: now.get(Calendar.MINUTE)
        fun pickTime(prefix: String) {
            TimePickerDialog(activity, { _, h, m ->
                finish(prompt.confirm(prefix + "%02d:%02d".format(h, m)))
            }, hour, minute, android.text.format.DateFormat.is24HourFormat(activity)).apply {
                setOnDismissListener { finish(prompt.dismiss()) }
            }.show()
        }
        if (type == DateTimePrompt.Type.TIME) {
            pickTime("")
        } else {
            val picker = DatePickerDialog(activity, { _, y, m, d ->
                val picked = "%04d-%02d-%02d".format(y, m + 1, d)
                if (type == DateTimePrompt.Type.DATETIME_LOCAL) pickTime("${picked}T") else finish(prompt.confirm(picked))
            }, year, month, day)
            picker.setButton(AlertDialog.BUTTON_NEUTRAL, activity.getString(R.string.prompt_clear)) { _, _ -> finish(prompt.confirm("")) }
            // Leaving for the time picker is not a dismissal.
            picker.setOnCancelListener { finish(prompt.dismiss()) }
            picker.show()
        }
        return result
    }

    override fun onBeforeUnloadPrompt(session: GeckoSession, prompt: BeforeUnloadPrompt) = ask(prompt) { answer ->
        setMessage(R.string.prompt_leave_page)
        setPositiveButton(R.string.prompt_leave) { _, _ -> answer(prompt.confirm(AllowOrDeny.ALLOW)) }
        setNegativeButton(R.string.prompt_stay) { _, _ -> answer(prompt.confirm(AllowOrDeny.DENY)) }
    }

    override fun onRepostConfirmPrompt(session: GeckoSession, prompt: RepostConfirmPrompt) = ask(prompt) { answer ->
        setMessage(R.string.prompt_resend)
        setPositiveButton(R.string.prompt_resend_action) { _, _ -> answer(prompt.confirm(AllowOrDeny.ALLOW)) }
        setNegativeButton(android.R.string.cancel) { _, _ -> answer(prompt.confirm(AllowOrDeny.DENY)) }
    }

    /** Pop-ups a page opens without a tap are blocked, as before. */
    override fun onPopupPrompt(session: GeckoSession, prompt: PopupPrompt): GeckoResult<PromptResponse> =
        GeckoResult.fromValue(prompt.confirm(AllowOrDeny.DENY))

    /** `navigator.share()` hands the text and link to Android's share sheet. */
    override fun onSharePrompt(session: GeckoSession, prompt: SharePrompt): GeckoResult<PromptResponse> {
        val text = listOfNotNull(prompt.text?.takeIf { it.isNotBlank() }, prompt.uri?.takeIf { it.isNotBlank() }).joinToString("\n")
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
        prompt.title?.let { send.putExtra(Intent.EXTRA_TITLE, it) }
        return try {
            activity.startActivity(Intent.createChooser(send, null))
            GeckoResult.fromValue(prompt.confirm(SharePrompt.Result.SUCCESS))
        } catch (e: android.content.ActivityNotFoundException) {
            GeckoResult.fromValue(prompt.confirm(SharePrompt.Result.FAILURE))
        }
    }

    override fun onFilePrompt(session: GeckoSession, prompt: FilePrompt): GeckoResult<PromptResponse> = activity.chooseFiles(prompt)
}
