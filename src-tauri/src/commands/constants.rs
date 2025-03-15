use lazy_static::*;
use phf::phf_set;
use regex::Regex;

lazy_static! {
    pub static ref URL_REGEX: Regex= Regex::new(
            r"https?://(?:[a-zA-Z0-9\-]+(?:\.[a-zA-Z0-9\-]+){1,}|localhost)(?::\d+)?(?:/[a-zA-Z0-9\-._~%!$&()*+;=:@/\?,\[\]#'{}|]*)?",
        ).expect("Failed to compile URL regex");
}

// Keywords related to time that appear in user conversations
// Automatically add the current system time as context to the conversation
pub static TIME_IND: phf::Set<&'static str> = phf_set! {
    "早上", "凌晨", "傍晚", "深夜", "下午", "上午", "中午", "晚上",
    "今天", "明天","昨天", "前天", "后天",
    "这个月", "上个月", "上月","本月","下月","下个月", "月初", "月底",
    "本年", "今年", "去年", "前年","下一年", "上一年","明年", "后年", "年初", "年底","下年", "上年",
    "下周", "星期","周末",
    "春节", "元旦", "国庆", "中秋", "端午", "清明", "工作日", "假期", "节日", "季节", "春天", "夏天", "秋天", "冬天","长假",
    "现在", "立刻", "马上", "稍后", "一会儿", "刚才", "之前", "之后", "未来", "过去", "最近", "几小时", "几分钟", "几秒",

    // 繁体版
    "後天","這個月", "上個月",  "下個月", "後年","下週", "週末",
    "春節",  "國慶",  "節日", "季節",  "長假",
    "現在", "馬上", "稍後", "一會兒", "剛才","之後", "未來", "過去", "幾小時", "幾分鐘", "幾秒",

    // 英文
    "morning", "dawn", "evening", "midnight", "afternoon", "noon", "night",
    "today", "tomorrow", "yesterday", "day before yesterday", "day after tomorrow",
    "this month", "last month", "next month", "beginning of the month", "end of the month",
    "this year", "last year", "next year", "beginning of the year", "end of the year",
    "next week", "week", "weekend",
    "Spring Festival", "New Year's Day", "National Day", "Mid-Autumn Festival", "Dragon Boat Festival", "Qingming Festival", "workday", "holiday", "festival", "season", "spring", "summer", "autumn", "winter", "long holiday",
    "now", "immediately", "right away", "later", "a while", "just now", "before", "after", "future", "past", "recently", "a few hours", "a few minutes", "a few seconds",

    // 日文
    "朝", "夜明け", "夕方", "真夜中", "午後", "午前",
    "今日", "明日", "昨日", "一昨日", "明後日",
    "今月", "先月", "来月", "月初め", "月末",
    "来年", "年初め", "年末",
    "来週", "週",
    "国慶節", "中秋節", "端午節", "清明節", "平日", "休み", "祝日", "長期休暇",
    "すぐ", "すぐに", "後で", "少し", "さっき", "数時間", "数分", "数秒"
};
